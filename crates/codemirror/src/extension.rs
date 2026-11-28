//! Typed editor extensions and transaction effects that configure them.

use std::{
    any::Any,
    fmt,
    marker::PhantomData,
    ops::Deref,
    sync::{
        Arc, LazyLock,
        atomic::{AtomicUsize, Ordering},
    },
};

use crate::{ChangeDesc, EditorSelection, StateEffect, StateEffectType, Text, Transaction};

mod registry;
pub(crate) use registry::{Extensions, InitialConfiguration, Reconfiguration, ResolvedValues};

type Key = usize;
type Value = Arc<dyn Any + Send + Sync>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct FacetId(Key);

fn next_key() -> Key {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    NEXT.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |key| {
        key.checked_add(1)
    })
    .expect("extension keys exhausted")
}

/// An invalid configuration. Errors reject state creation or the complete transaction.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConfigurationError {
    DuplicateCompartment,
    DependencyCycle,
    MissingField,
    InvalidFieldData,
    InvalidValueType,
    /// A strict language parser rejected the final text of this update.
    StrictSyntax(crate::language::lezer::ParseError),
    /// A facet needed to create the initial document or selection has a computed input.
    DynamicInitialPolicy,
}

trait Values {
    fn get(&self, key: Key) -> Result<Option<Value>, ConfigurationError>;

    fn is_reconfiguration_preview(&self) -> bool {
        false
    }
}

/// A read-only view of state available to a field or facet computation.
#[derive(Clone, Copy)]
pub struct StateContext<'a> {
    text: &'a Text,
    selection: &'a EditorSelection,
    values: &'a dyn Values,
}

impl StateContext<'_> {
    pub fn text(&self) -> &Text {
        self.text
    }
    pub fn selection(&self) -> &EditorSelection {
        self.selection
    }

    pub(crate) fn is_reconfiguration_preview(&self) -> bool {
        self.values.is_reconfiguration_preview()
    }

    /// Read a field by definition identity. A field that is not enabled returns `None`.
    pub fn field<T: Send + Sync + 'static>(
        &self,
        field: &StateFieldReader<T>,
    ) -> Result<Option<Arc<T>>, ConfigurationError> {
        self.values.get(field.key)?.map(from_value).transpose()
    }

    /// Combine the enabled inputs of a facet, or return its default.
    pub fn facet<O: Send + Sync + 'static>(
        &self,
        facet: &FacetReader<O>,
    ) -> Result<Arc<O>, ConfigurationError> {
        self.values
            .get(facet.key)?
            .map(from_value)
            .transpose()
            .map(|value| value.unwrap_or_else(|| facet.default.clone()))
    }
}

fn from_value<T: Send + Sync + 'static>(value: Value) -> Result<Arc<T>, ConfigurationError> {
    value
        .downcast()
        .map_err(|_| ConfigurationError::InvalidValueType)
}

/// Both state snapshots and transaction data for a field update.
pub struct StateUpdate<'a> {
    start: StateContext<'a>,
    state: StateContext<'a>,
    transaction: &'a Transaction,
    changes: &'a ChangeDesc,
}

impl StateUpdate<'_> {
    pub fn start(&self) -> &StateContext<'_> {
        &self.start
    }
    pub fn state(&self) -> &StateContext<'_> {
        &self.state
    }
    pub fn transaction(&self) -> &Transaction {
        self.transaction
    }
    pub fn changes(&self) -> &ChangeDesc {
        self.changes
    }
}

type Initialize = dyn Fn(&StateContext<'_>) -> Result<Value, ConfigurationError> + Send + Sync;
type Update =
    dyn Fn(Value, &StateUpdate<'_>) -> Result<(Value, bool), ConfigurationError> + Send + Sync;
type Compare<T> = dyn Fn(&T, &T) -> bool + Send + Sync;
type Provide<T> = dyn FnOnce(StateFieldReader<T>) -> Extension;

struct FieldDefinition {
    key: Key,
    create: Box<Initialize>,
    update: Box<Update>,
    provides: Option<Extension>,
}

/// Options for a typed state field. By default the field has no comparator
/// and does not provide extensions.
pub struct StateFieldOptions<T> {
    compare: Option<Box<Compare<T>>>,
    provide: Option<Box<Provide<T>>>,
}

impl<T> Default for StateFieldOptions<T> {
    fn default() -> Self {
        Self {
            compare: None,
            provide: None,
        }
    }
}

impl<T> StateFieldOptions<T> {
    /// Keep the previous value when this function returns `true` for the
    /// previous value and the result of an update, in that order.
    pub fn compare(mut self, compare: impl Fn(&T, &T) -> bool + Send + Sync + 'static) -> Self {
        self.compare = Some(Box::new(compare));
        self
    }

    /// Run once when the field is defined. The resulting extension is enabled
    /// with the field and inherits its precedence unless it sets its own.
    pub fn provide(
        mut self,
        provide: impl FnOnce(StateFieldReader<T>) -> Extension + 'static,
    ) -> Self {
        self.provide = Some(Box::new(provide));
        self
    }
}

/// A typed field identity for reading a value without owning its definition.
pub struct StateFieldReader<T> {
    key: Key,
    marker: PhantomData<fn() -> T>,
}

impl<T> Copy for StateFieldReader<T> {}

impl<T> Clone for StateFieldReader<T> {
    fn clone(&self) -> Self {
        *self
    }
}

/// A typed field that computes a value at initialization and on each transaction.
pub struct StateField<T> {
    definition: Arc<FieldDefinition>,
    reader: StateFieldReader<T>,
}

impl<T> Clone for StateField<T> {
    fn clone(&self) -> Self {
        Self {
            definition: self.definition.clone(),
            reader: self.reader,
        }
    }
}

impl<T> Deref for StateField<T> {
    type Target = StateFieldReader<T>;

    fn deref(&self) -> &Self::Target {
        &self.reader
    }
}

impl<T> From<StateField<T>> for StateFieldReader<T> {
    fn from(field: StateField<T>) -> Self {
        field.reader()
    }
}

impl<T> StateField<T> {
    /// Read this field without retaining its definition or provided extensions.
    pub fn reader(&self) -> StateFieldReader<T> {
        self.reader
    }
}

impl<T: Send + Sync + 'static> StateField<T> {
    /// Field callbacks must not mutate other state. The resolver can reject a transaction.
    pub fn new(
        create: impl Fn(&StateContext<'_>) -> Result<T, ConfigurationError> + Send + Sync + 'static,
        update: impl Fn(&T, &StateUpdate<'_>) -> Result<T, ConfigurationError> + Send + Sync + 'static,
    ) -> Self {
        Self::define(create, update, StateFieldOptions::default())
    }

    /// Define a field with options. To reuse an allocation without a
    /// comparator, use [`Self::define_shared`].
    pub fn define(
        create: impl Fn(&StateContext<'_>) -> Result<T, ConfigurationError> + Send + Sync + 'static,
        update: impl Fn(&T, &StateUpdate<'_>) -> Result<T, ConfigurationError> + Send + Sync + 'static,
        options: StateFieldOptions<T>,
    ) -> Self {
        Self::define_shared(
            move |state| create(state).map(Arc::new),
            move |previous, transaction| update(previous, transaction).map(Arc::new),
            options,
        )
    }

    /// Define a field whose callbacks return shared values. An update can
    /// return a clone of the previous `Arc` to keep the same allocation.
    pub fn define_shared(
        create: impl Fn(&StateContext<'_>) -> Result<Arc<T>, ConfigurationError> + Send + Sync + 'static,
        update: impl Fn(&Arc<T>, &StateUpdate<'_>) -> Result<Arc<T>, ConfigurationError>
        + Send
        + Sync
        + 'static,
        options: StateFieldOptions<T>,
    ) -> Self {
        let key = next_key();
        let reader = StateFieldReader {
            key,
            marker: PhantomData,
        };
        let StateFieldOptions { compare, provide } = options;
        Self {
            definition: Arc::new(FieldDefinition {
                key,
                create: Box::new(move |state| Ok(create(state)?)),
                update: Box::new(move |previous, transaction| {
                    let previous = from_value::<T>(previous)?;
                    let next = update(&previous, transaction)?;
                    let equal = compare.as_ref().map_or_else(
                        || Arc::ptr_eq(&previous, &next),
                        |equal| equal(&previous, &next),
                    );
                    if equal {
                        Ok((previous, false))
                    } else {
                        Ok((next, true))
                    }
                }),
                provides: provide.map(|provide| provide(reader)),
            }),
            reader,
        }
    }

    pub fn extension(&self) -> Extension {
        Extension::with_key(
            self.definition.key,
            ExtensionKind::Field(self.definition.clone()),
        )
    }

    /// Enable this field with a custom initial value. The highest precedence
    /// initializer wins. Reuse this extension to retain the value on reconfiguration.
    pub fn init(
        &self,
        create: impl Fn(&StateContext<'_>) -> Result<T, ConfigurationError> + Send + Sync + 'static,
    ) -> Extension {
        Extension::group([
            self.extension(),
            FIELD_INITIALIZERS.of(FieldInitializer {
                field: self.definition.key,
                create: Arc::new(move |state| Ok(Arc::new(create(state)?))),
            }),
        ])
    }
}

#[derive(Clone)]
struct FieldInitializer {
    field: Key,
    create: Arc<Initialize>,
}

type Initializers = Facet<FieldInitializer, Vec<FieldInitializer>>;
static FIELD_INITIALIZERS: LazyLock<Initializers> = LazyLock::new(Facet::collect);

fn initializer(
    context: &StateContext<'_>,
    key: Key,
) -> Result<Option<Arc<Initialize>>, ConfigurationError> {
    Ok(context
        .facet(&FIELD_INITIALIZERS)?
        .iter()
        .find(|item| item.field == key)
        .map(|item| item.create.clone()))
}

type Combine = dyn Fn(&[Value]) -> Result<Value, ConfigurationError> + Send + Sync;
type Compute = dyn Fn(&StateContext<'_>) -> Result<Vec<Value>, ConfigurationError> + Send + Sync;
type CompareValue = dyn Fn(&Value, &Value) -> Result<bool, ConfigurationError> + Send + Sync;
type Enable<'a, O> = dyn FnOnce(FacetReader<O>) -> Extension + 'a;

fn value_ref<T: Send + Sync + 'static>(value: &Value) -> Result<&T, ConfigurationError> {
    value
        .downcast_ref()
        .ok_or(ConfigurationError::InvalidValueType)
}

fn erase_compare<T: Send + Sync + 'static>(compare: Box<Compare<T>>) -> Box<CompareValue> {
    Box::new(move |next, previous| Ok(compare(value_ref(next)?, value_ref(previous)?)))
}

struct FacetDefinition {
    key: Key,
    default: Value,
    combine: Box<Combine>,
    compare_input: Option<Box<CompareValue>>,
    compare_output: FacetComparison,
    enables: Option<Extension>,
}

enum FacetComparison {
    Identity,
    Inputs,
    Custom(Box<CompareValue>),
}

struct FacetInput {
    key: Key,
    definition: Arc<FacetDefinition>,
    source: InputSource,
}

enum InputSource {
    Static(Value),
    Dynamic {
        compute: Box<Compute>,
        dependencies: Option<FacetDependencies>,
    },
}

/// Comparison and extension settings for [`Facet::define`] and [`Facet::collect_with`].
pub struct FacetOptions<'a, I, O> {
    compare_input: Option<Box<Compare<I>>>,
    compare_output: Option<Box<Compare<O>>>,
    enables: Option<Box<Enable<'a, O>>>,
}

impl<'a, I, O> FacetOptions<'a, I, O> {
    /// Compare a new computed input with its cached value, in that order.
    /// Return `true` to keep the cached value. For a multi-input provider,
    /// the lists must have the same length and all pairs must compare equal.
    pub fn compare_input(&mut self, compare: impl Fn(&I, &I) -> bool + Send + Sync + 'static) {
        self.compare_input = Some(Box::new(compare));
    }

    /// Compare a new combined output with its cached value, in that order.
    /// Return `true` to keep the cached output and not invalidate its dependents.
    pub fn compare(&mut self, compare: impl Fn(&O, &O) -> bool + Send + Sync + 'static) {
        self.compare_output = Some(Box::new(compare));
    }

    /// Run once at definition. Each input enables the resulting extension
    /// at default precedence, independent of the input's precedence.
    pub fn enables(&mut self, enables: impl FnOnce(FacetReader<O>) -> Extension + 'a) {
        self.enables = Some(Box::new(enables));
    }
}

/// Typed dependencies for a computed facet input. An empty list means that
/// the input is computed only when its extension is first enabled.
#[derive(Default)]
pub struct FacetDependencies(Vec<Dependency>);

impl FacetDependencies {
    /// Invalidate on a document change.
    pub fn document(&mut self) {
        self.0.push(Dependency::Document);
    }

    /// Invalidate on a document change or an explicit selection, even if it
    /// equals the previous selection.
    pub fn selection(&mut self) {
        self.0.push(Dependency::Selection);
    }

    /// Invalidate when the field's cached value changes.
    pub fn field<T>(&mut self, reader: &StateFieldReader<T>) {
        self.0.push(Dependency::Field(reader.key));
    }

    /// Invalidate when the facet's cached output changes.
    pub fn facet<O: Send + Sync + 'static>(&mut self, reader: &FacetReader<O>) {
        self.0
            .push(Dependency::Facet(reader.key, reader.default.clone()));
    }
}

enum Dependency {
    Document,
    Selection,
    Field(Key),
    Facet(Key, Value),
}

/// A typed facet identity for reading an output without owning its definition.
pub struct FacetReader<O> {
    key: Key,
    default: Arc<O>,
}

impl<O> Clone for FacetReader<O> {
    fn clone(&self) -> Self {
        Self {
            key: self.key,
            default: self.default.clone(),
        }
    }
}

/// A typed input collection with an output computed in precedence order.
pub struct Facet<I, O> {
    definition: Arc<FacetDefinition>,
    reader: FacetReader<O>,
    marker: PhantomData<fn() -> I>,
}

impl<I, O> Clone for Facet<I, O> {
    fn clone(&self) -> Self {
        Self {
            definition: self.definition.clone(),
            reader: self.reader.clone(),
            marker: PhantomData,
        }
    }
}

impl<I, O> Deref for Facet<I, O> {
    type Target = FacetReader<O>;

    fn deref(&self) -> &Self::Target {
        &self.reader
    }
}

impl<I, O> Facet<I, O> {
    pub(crate) fn id(&self) -> FacetId {
        FacetId(self.reader.key)
    }

    /// Read this facet without retaining its definition or enabled extensions.
    pub fn reader(&self) -> FacetReader<O> {
        self.reader.clone()
    }
}

impl<I: Clone + Send + Sync + 'static> Facet<I, Vec<I>> {
    pub fn collect() -> Self {
        Self::collect_with(|_| {})
    }

    /// Collect inputs in precedence order. Without an output comparator,
    /// reuse the output when the ordered input allocations are the same.
    pub fn collect_with<'a>(configure: impl FnOnce(&mut FacetOptions<'a, I, Vec<I>>)) -> Self {
        Self::build(
            |inputs| inputs.iter().map(|input| (*input).clone()).collect(),
            configure,
            FacetComparison::Inputs,
        )
    }
}

impl<I: Send + Sync + 'static, O: Send + Sync + 'static> Facet<I, O> {
    pub fn new(combine: impl Fn(&[&I]) -> O + Send + Sync + 'static) -> Self {
        Self::define(combine, |_| {})
    }

    /// Run `enables` once to construct an extension. Each input enables it
    /// at default precedence, regardless of the input's precedence.
    pub fn with_enables(
        combine: impl Fn(&[&I]) -> O + Send + Sync + 'static,
        enables: impl FnOnce(FacetReader<O>) -> Extension,
    ) -> Self {
        Self::define(combine, |options| options.enables(enables))
    }

    /// Define a facet with optional input and output comparators. Without a
    /// comparator, equality uses allocation identity, not `PartialEq`.
    /// Set a comparator for values that must compare equal across allocations.
    pub fn define<'a>(
        combine: impl Fn(&[&I]) -> O + Send + Sync + 'static,
        configure: impl FnOnce(&mut FacetOptions<'a, I, O>),
    ) -> Self {
        Self::build(combine, configure, FacetComparison::Identity)
    }

    fn build<'a>(
        combine: impl Fn(&[&I]) -> O + Send + Sync + 'static,
        configure: impl FnOnce(&mut FacetOptions<'a, I, O>),
        default_comparison: FacetComparison,
    ) -> Self {
        let mut options = FacetOptions {
            compare_input: None,
            compare_output: None,
            enables: None,
        };
        configure(&mut options);
        let key = next_key();
        let reader = FacetReader {
            key,
            default: Arc::new(combine(&[])),
        };
        Self {
            definition: Arc::new(FacetDefinition {
                key,
                default: reader.default.clone(),
                combine: Box::new(move |values| {
                    let inputs = values
                        .iter()
                        .map(value_ref)
                        .collect::<Result<Vec<_>, _>>()?;
                    Ok(Arc::new(combine(&inputs)))
                }),
                compare_input: options.compare_input.map(erase_compare),
                compare_output: options
                    .compare_output
                    .map_or(default_comparison, |compare| {
                        FacetComparison::Custom(erase_compare(compare))
                    }),
                enables: options.enables.map(|enables| enables(reader.clone())),
            }),
            reader,
            marker: PhantomData,
        }
    }

    pub fn of(&self, value: I) -> Extension {
        self.input(InputSource::Static(Arc::new(value)))
    }

    /// Compute on every transaction and reconfiguration. To declare the
    /// exact dependencies, use [`Self::compute_with_dependencies`].
    pub fn compute(
        &self,
        compute: impl Fn(&StateContext<'_>) -> Result<I, ConfigurationError> + Send + Sync + 'static,
    ) -> Extension {
        self.compute_input(None, compute)
    }

    /// Compute on creation and when a declared dependency invalidates the
    /// input. Declare all document, selection, field, and facet reads.
    pub fn compute_with_dependencies(
        &self,
        declare: impl FnOnce(&mut FacetDependencies),
        compute: impl Fn(&StateContext<'_>) -> Result<I, ConfigurationError> + Send + Sync + 'static,
    ) -> Extension {
        let mut dependencies = FacetDependencies::default();
        declare(&mut dependencies);
        self.compute_input(Some(dependencies), compute)
    }

    fn compute_input(
        &self,
        dependencies: Option<FacetDependencies>,
        compute: impl Fn(&StateContext<'_>) -> Result<I, ConfigurationError> + Send + Sync + 'static,
    ) -> Extension {
        self.input(InputSource::Dynamic {
            compute: Box::new(move |state| Ok(vec![Arc::new(compute(state)?)])),
            dependencies,
        })
    }

    /// Compute zero or more inputs on every transaction and reconfiguration.
    pub fn compute_many(
        &self,
        compute: impl Fn(&StateContext<'_>) -> Result<Vec<I>, ConfigurationError>
        + Send
        + Sync
        + 'static,
    ) -> Extension {
        self.compute_many_inputs(None, compute)
    }

    /// Compute zero or more inputs with the same invalidation rules as
    /// [`Self::compute_with_dependencies`].
    pub fn compute_many_with_dependencies(
        &self,
        declare: impl FnOnce(&mut FacetDependencies),
        compute: impl Fn(&StateContext<'_>) -> Result<Vec<I>, ConfigurationError>
        + Send
        + Sync
        + 'static,
    ) -> Extension {
        let mut dependencies = FacetDependencies::default();
        declare(&mut dependencies);
        self.compute_many_inputs(Some(dependencies), compute)
    }

    fn compute_many_inputs(
        &self,
        dependencies: Option<FacetDependencies>,
        compute: impl Fn(&StateContext<'_>) -> Result<Vec<I>, ConfigurationError>
        + Send
        + Sync
        + 'static,
    ) -> Extension {
        self.input(InputSource::Dynamic {
            compute: Box::new(move |state| {
                Ok(compute(state)?
                    .into_iter()
                    .map(|input| Arc::new(input) as Value)
                    .collect())
            }),
            dependencies,
        })
    }

    /// Compute an input from one field. Reuse the result until the field
    /// changes. The field must be enabled while this input is enabled.
    pub fn from_field<T: Send + Sync + 'static>(
        &self,
        field: impl Into<StateFieldReader<T>>,
        input: impl Fn(&T) -> I + Send + Sync + 'static,
    ) -> Extension {
        let field = field.into();
        self.compute_with_dependencies(
            |deps| deps.field(&field),
            move |state| {
                let value = state
                    .field(&field)?
                    .ok_or(ConfigurationError::MissingField)?;
                Ok(input(&value))
            },
        )
    }

    fn input(&self, source: InputSource) -> Extension {
        let key = next_key();
        Extension::with_key(
            key,
            ExtensionKind::Facet(Arc::new(FacetInput {
                key,
                definition: self.definition.clone(),
                source,
            })),
        )
    }
}

/// Extends the order in which editor extensions contribute inputs.
#[derive(Clone, Copy, Debug, Default, Ord, PartialOrd, Eq, PartialEq)]
pub enum Precedence {
    Highest,
    High,
    #[default]
    Default,
    Low,
    Lowest,
}

/// A reusable part of the editor configuration.
#[derive(Clone, Debug)]
pub struct Compartment(Key);

impl Compartment {
    pub fn new() -> Self {
        Self(next_key())
    }

    pub fn of(&self, extension: Extension) -> Extension {
        Extension::new(ExtensionKind::Compartment(self.0, extension))
    }

    pub fn reconfigure(&self, extension: Extension) -> StateEffect {
        CONFIGURATION.of(ConfigurationChange::Compartment(self.0, extension))
    }
}

impl Default for Compartment {
    fn default() -> Self {
        Self::new()
    }
}

struct ExtensionNode {
    key: Key,
    kind: ExtensionKind,
}

enum ExtensionKind {
    Group(Vec<Extension>),
    Priority(Precedence, Extension),
    Compartment(Key, Extension),
    Field(Arc<FieldDefinition>),
    Facet(Arc<FacetInput>),
}

/// A field, facet input, compartment, or group of extensions.
#[derive(Clone)]
pub struct Extension(Arc<ExtensionNode>);

impl Extension {
    fn with_key(key: Key, kind: ExtensionKind) -> Self {
        Self(Arc::new(ExtensionNode { key, kind }))
    }
    fn new(kind: ExtensionKind) -> Self {
        Self::with_key(next_key(), kind)
    }

    pub fn group(extensions: impl IntoIterator<Item = Extension>) -> Self {
        Self::new(ExtensionKind::Group(extensions.into_iter().collect()))
    }

    pub fn with_precedence(self, precedence: Precedence) -> Self {
        Self::new(ExtensionKind::Priority(precedence, self))
    }

    /// Replace the root configuration as part of a transaction.
    pub fn reconfigure(self) -> StateEffect {
        CONFIGURATION.of(ConfigurationChange::Replace(self))
    }

    /// Add to the root configuration as part of a transaction.
    pub fn append(self) -> StateEffect {
        CONFIGURATION.of(ConfigurationChange::Append(self))
    }
}

impl Default for Extension {
    fn default() -> Self {
        Self::group([])
    }
}

impl fmt::Debug for Extension {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Extension").field(&self.0.key).finish()
    }
}

#[derive(Clone)]
enum ConfigurationChange {
    Replace(Extension),
    Append(Extension),
    Compartment(Key, Extension),
}

static CONFIGURATION: LazyLock<StateEffectType<ConfigurationChange>> =
    LazyLock::new(StateEffectType::new);
