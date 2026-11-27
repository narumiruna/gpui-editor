use std::{
    cell::RefCell,
    collections::{BTreeMap, HashMap, HashSet},
    fmt,
    sync::Arc,
};

use super::{
    CONFIGURATION, CompareValue, Compartment, ConfigurationChange, ConfigurationError, Dependency,
    Extension, ExtensionKind, FacetComparison, FacetInput, FacetReader, FieldDefinition,
    InputSource, Key, Precedence, StateContext, StateUpdate, Value, Values, from_value,
    initializer,
};
use crate::{ChangeDesc, EditorSelection, Text, Transaction};

#[derive(Clone, Default)]
pub(crate) struct Extensions(Option<Arc<Snapshot>>);

struct Snapshot {
    config: Arc<Configuration>,
    values: HashMap<Key, Resolved>,
}

/// A flattened prospective configuration. No callbacks have run for it yet.
pub(crate) struct Reconfiguration(Configuration);

impl Reconfiguration {
    pub(crate) fn has_facet<O>(&self, facet: &FacetReader<O>) -> bool {
        self.0.slots.contains_key(&facet.key)
    }
}

#[derive(Clone)]
struct Resolved {
    value: Cached,
    changed: bool,
}

#[derive(Clone)]
enum Cached {
    Field(Value),
    Input(Arc<[Value]>),
    Facet {
        output: Value,
        inputs: Option<Arc<[Value]>>,
    },
}

impl Cached {
    fn facet(output: Value, values: Vec<Value>, comparison: &FacetComparison) -> Self {
        Self::Facet {
            output,
            inputs: matches!(comparison, FacetComparison::Inputs).then(|| values.into()),
        }
    }

    fn state_value(&self) -> Option<Value> {
        match self {
            Self::Field(value) | Self::Facet { output: value, .. } => Some(value.clone()),
            Self::Input(_) => None,
        }
    }
}

impl fmt::Debug for Extensions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let fields = self.0.as_ref().map_or(0, |snapshot| snapshot.values.len());
        f.debug_struct("Extensions")
            .field("slots", &fields)
            .finish()
    }
}

impl Values for Extensions {
    fn get(&self, key: Key) -> Result<Option<Value>, ConfigurationError> {
        Ok(self.cached(key).and_then(Cached::state_value))
    }
}

/// The flattened configuration and its static facet outputs before the initial state exists.
pub(crate) struct InitialConfiguration {
    config: Configuration,
    values: HashMap<Key, Resolved>,
}

impl InitialConfiguration {
    pub(crate) fn new(root: Extension) -> Result<Self, ConfigurationError> {
        let config = Configuration::new(root, &HashMap::new())?;
        let mut values = HashMap::new();
        for &key in &config.order[..config.static_count] {
            let Slot::Facet(providers) = &config.slots[&key] else {
                unreachable!("static facet slot");
            };
            let inputs = providers
                .iter()
                .map(|input| match &input.source {
                    InputSource::Static(value) => value.clone(),
                    InputSource::Dynamic { .. } => unreachable!("static facet input"),
                })
                .collect::<Vec<_>>();
            let definition = &providers[0].definition;
            let output = (definition.combine)(&inputs)?;
            values.insert(
                key,
                Resolved {
                    value: Cached::facet(output, inputs, &definition.compare_output),
                    changed: true,
                },
            );
        }
        Ok(Self { config, values })
    }

    pub(crate) fn static_facet<O: Send + Sync + 'static>(
        &self,
        facet: &FacetReader<O>,
    ) -> Result<Arc<O>, ConfigurationError> {
        match self.values.get(&facet.key) {
            Some(value) => from_value(value.value.state_value().expect("static facet output")),
            None if self.config.slots.contains_key(&facet.key) => {
                Err(ConfigurationError::DynamicInitialPolicy)
            }
            None => Ok(facet.default.clone()),
        }
    }

    pub(crate) fn finish(
        self,
        text: &Text,
        selection: &EditorSelection,
    ) -> Result<Extensions, ConfigurationError> {
        Extensions::evaluate(
            Arc::new(self.config),
            text,
            selection,
            Evaluation::Create,
            self.values,
        )
    }
}

impl Extensions {
    fn cached(&self, key: Key) -> Option<&Cached> {
        self.0.as_ref()?.values.get(&key).map(|slot| &slot.value)
    }

    fn slot(&self, key: Key) -> Option<&Slot> {
        self.0.as_ref()?.config.slots.get(&key)
    }

    pub(crate) fn create(
        root: Extension,
        text: &Text,
        selection: &EditorSelection,
    ) -> Result<Self, ConfigurationError> {
        let config = Arc::new(Configuration::new(root, &HashMap::new())?);
        Self::evaluate(config, text, selection, Evaluation::Create, HashMap::new())
    }

    pub(crate) fn context<'a>(
        &'a self,
        text: &'a Text,
        selection: &'a EditorSelection,
    ) -> StateContext<'a> {
        StateContext {
            text,
            selection,
            values: self,
        }
    }

    pub(crate) fn compartment(&self, compartment: &Compartment) -> Option<&Extension> {
        self.0.as_ref()?.config.compartments.get(&compartment.0)
    }

    pub(crate) fn active(&self) -> bool {
        self.0.is_some()
    }

    pub(crate) fn prepare_reconfiguration(
        &self,
        transaction: &Transaction,
    ) -> Result<Option<Reconfiguration>, ConfigurationError> {
        let mut configuration = None;
        for change in transaction
            .effects()
            .iter()
            .filter_map(|effect| effect.get(&CONFIGURATION))
        {
            let (root, compartments) = configuration.get_or_insert_with(|| {
                self.0.as_ref().map_or_else(
                    || (Extension::default(), HashMap::new()),
                    |current| {
                        (
                            current.config.root.clone(),
                            current.config.compartments.clone(),
                        )
                    },
                )
            });
            match change {
                ConfigurationChange::Replace(extension) => *root = extension.clone(),
                ConfigurationChange::Append(extension) => {
                    *root = Extension::group([root.clone(), extension.clone()])
                }
                ConfigurationChange::Compartment(key, extension) => {
                    compartments.insert(*key, extension.clone());
                }
            }
        }
        configuration
            .map(|(root, compartments)| {
                Configuration::new(root, &compartments).map(Reconfiguration)
            })
            .transpose()
    }

    pub(crate) fn update<'a>(
        &self,
        old: (&'a Text, &'a EditorSelection),
        new: (&Text, &EditorSelection),
        transaction: &'a Transaction,
        changes: &'a ChangeDesc,
        reconfiguration: Option<Reconfiguration>,
    ) -> Result<Self, ConfigurationError> {
        let start = self.context(old.0, old.1);
        let intermediate;
        let (config, start_values) = match reconfiguration {
            Some(Reconfiguration(config)) => {
                let config = Arc::new(config);
                intermediate = Self::evaluate(
                    config.clone(),
                    old.0,
                    old.1,
                    Evaluation::Reconfigure {
                        start,
                        previous: self,
                    },
                    HashMap::new(),
                )?;
                (config, &intermediate)
            }
            None => match &self.0 {
                Some(snapshot) => (snapshot.config.clone(), self),
                None => return Ok(Self::default()),
            },
        };
        Self::evaluate(
            config,
            new.0,
            new.1,
            Evaluation::Update {
                start,
                start_values,
                transaction,
                changes,
            },
            HashMap::new(),
        )
    }

    fn evaluate(
        config: Arc<Configuration>,
        text: &Text,
        selection: &EditorSelection,
        phase: Evaluation<'_>,
        values: HashMap<Key, Resolved>,
    ) -> Result<Self, ConfigurationError> {
        let resolver = Resolver {
            config: &config,
            text,
            selection,
            phase,
            resolved: RefCell::new(values),
            visiting: RefCell::new(HashSet::new()),
        };
        for &slot in &config.order {
            resolver.resolve(slot)?;
        }
        let values = resolver.resolved.into_inner();
        Ok(Self(Some(Arc::new(Snapshot { config, values }))))
    }
}

enum Evaluation<'a> {
    Create,
    Reconfigure {
        start: StateContext<'a>,
        previous: &'a Extensions,
    },
    Update {
        start: StateContext<'a>,
        start_values: &'a Extensions,
        transaction: &'a Transaction,
        changes: &'a ChangeDesc,
    },
}

struct Resolver<'a> {
    config: &'a Configuration,
    text: &'a Text,
    selection: &'a EditorSelection,
    phase: Evaluation<'a>,
    resolved: RefCell<HashMap<Key, Resolved>>,
    visiting: RefCell<HashSet<Key>>,
}

impl Values for Resolver<'_> {
    fn get(&self, key: Key) -> Result<Option<Value>, ConfigurationError> {
        Ok(self.resolve(key)?.and_then(|slot| slot.value.state_value()))
    }
}

impl Resolver<'_> {
    fn resolve(&self, key: Key) -> Result<Option<Resolved>, ConfigurationError> {
        if let Some(value) = self.resolved.borrow().get(&key) {
            return Ok(Some(value.clone()));
        }
        let Some(slot) = self.config.slots.get(&key) else {
            return Ok(None);
        };
        if !self.visiting.borrow_mut().insert(key) {
            return Err(ConfigurationError::DependencyCycle);
        }
        let computed = self.compute(key, slot);
        self.visiting.borrow_mut().remove(&key);
        let (value, changed) = computed?;
        let resolved = Resolved { value, changed };
        self.resolved.borrow_mut().insert(key, resolved.clone());
        Ok(Some(resolved))
    }

    fn compute(&self, key: Key, slot: &Slot) -> Result<(Cached, bool), ConfigurationError> {
        let context = StateContext {
            text: self.text,
            selection: self.selection,
            values: self,
        };
        match slot {
            Slot::Field(field) => self
                .field(field, context)
                .map(|(value, changed)| (Cached::Field(value), changed)),
            Slot::Input(input) => self.input(input, context),
            Slot::Facet(providers) => self.facet(key, providers),
        }
    }

    fn field(
        &self,
        field: &FieldDefinition,
        state: StateContext<'_>,
    ) -> Result<(Value, bool), ConfigurationError> {
        let key = field.key;
        let value = match self.phase {
            Evaluation::Create => match initializer(&state, key)? {
                Some(create) => create(&state),
                None => (field.create)(&state),
            },
            Evaluation::Reconfigure { start, previous } => {
                let create = initializer(&state, key)?;
                if let Some(custom) = &create
                    && initializer(&start, key)?.is_none_or(|old| !Arc::ptr_eq(&old, custom))
                {
                    return custom(&state).map(|value| (value, true));
                }
                if let Some(value) = previous.get(key)? {
                    return Ok((value, false));
                }
                match create {
                    Some(create) => create(&state),
                    None => (field.create)(&state),
                }
            }
            Evaluation::Update {
                start,
                start_values,
                transaction,
                changes,
            } => {
                let value = start_values
                    .get(key)?
                    .ok_or(ConfigurationError::MissingField)?;
                return (field.update)(
                    value,
                    &StateUpdate {
                        start,
                        state,
                        transaction,
                        changes,
                    },
                );
            }
        }?;
        Ok((value, true))
    }

    fn input(
        &self,
        input: &FacetInput,
        state: StateContext<'_>,
    ) -> Result<(Cached, bool), ConfigurationError> {
        let InputSource::Dynamic {
            compute,
            dependencies,
        } = &input.source
        else {
            unreachable!("only computed inputs have slots")
        };
        let previous = match self.phase {
            Evaluation::Create => None,
            Evaluation::Reconfigure { previous, .. } => previous.cached(input.key),
            Evaluation::Update { start_values, .. } => start_values.cached(input.key),
        };
        if let Some(Cached::Input(old)) = previous {
            if let Some(dependencies) = dependencies {
                let invalid = match self.phase {
                    Evaluation::Create => unreachable!("new inputs have no cache"),
                    Evaluation::Reconfigure { previous, .. } => {
                        self.reconfigured(&dependencies.0, previous)?
                    }
                    Evaluation::Update {
                        transaction,
                        changes,
                        ..
                    } => self.invalidated(
                        &dependencies.0,
                        !changes.is_empty(),
                        transaction.selection().is_some(),
                    )?,
                };
                if !invalid {
                    return Ok((Cached::Input(old.clone()), false));
                }
            }
            let next = compute(&state)?;
            if same_inputs(&next, old, input.definition.compare_input.as_deref())? {
                return Ok((Cached::Input(old.clone()), false));
            }
            return Ok((Cached::Input(next.into()), true));
        }
        Ok((Cached::Input(compute(&state)?.into()), true))
    }

    fn invalidated(
        &self,
        dependencies: &[Dependency],
        doc: bool,
        selection: bool,
    ) -> Result<bool, ConfigurationError> {
        if dependencies.iter().any(|dependency| match dependency {
            Dependency::Document => doc,
            Dependency::Selection => doc || selection,
            _ => false,
        }) {
            return Ok(true);
        }
        let mut changed = false;
        for dependency in dependencies {
            if let Dependency::Field(key) | Dependency::Facet(key, _) = dependency {
                changed |= self.resolve(*key)?.is_some_and(|slot| slot.changed);
            }
        }
        Ok(changed)
    }

    fn reconfigured(
        &self,
        dependencies: &[Dependency],
        previous: &Extensions,
    ) -> Result<bool, ConfigurationError> {
        for dependency in dependencies {
            let (key, default) = match dependency {
                Dependency::Document | Dependency::Selection => continue,
                Dependency::Field(key) => (*key, None),
                Dependency::Facet(key, default) => (*key, Some(default)),
            };
            let old = previous.get(key)?;
            let next = self.get(key)?;
            let same = match (old.as_ref().or(default), next.as_ref().or(default)) {
                (Some(old), Some(next)) => Arc::ptr_eq(old, next),
                (None, None) => true,
                _ => false,
            };
            if !same {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn facet(
        &self,
        key: Key,
        providers: &[Arc<FacetInput>],
    ) -> Result<(Cached, bool), ConfigurationError> {
        let mut changed = false;
        for input in providers {
            if matches!(input.source, InputSource::Dynamic { .. }) {
                changed |= self.resolve(input.key)?.expect("input slot").changed;
            }
        }
        let previous = match self.phase {
            Evaluation::Create => None,
            Evaluation::Update { start_values, .. } => {
                let old = start_values.cached(key).expect("configured facet");
                if !changed {
                    return Ok((old.clone(), false));
                }
                Some(old)
            }
            Evaluation::Reconfigure { previous, .. } => {
                let old = previous.cached(key);
                if !changed
                    && let Some(old) = old
                    && let Some(Slot::Facet(old_providers)) = previous.slot(key)
                    && providers
                        .iter()
                        .map(|p| p.key)
                        .eq(old_providers.iter().map(|p| p.key))
                {
                    return Ok((old.clone(), false));
                }
                old
            }
        };
        let mut values = Vec::new();
        for input in providers {
            match &input.source {
                InputSource::Static(value) => values.push(value.clone()),
                InputSource::Dynamic { .. } => {
                    let resolved = self.resolved.borrow();
                    let Cached::Input(inputs) = &resolved[&input.key].value else {
                        unreachable!("input slot must contain inputs")
                    };
                    values.extend(inputs.iter().cloned());
                }
            }
        }
        let definition = &providers[0].definition;
        let output = (definition.combine)(&values)?;
        let (old_output, old_inputs) = match previous {
            Some(Cached::Facet { output, inputs }) => {
                (output, inputs.as_deref().unwrap_or_default())
            }
            _ => (&definition.default, &[][..]),
        };
        let equal = match self.phase {
            Evaluation::Create => false,
            _ => match &definition.compare_output {
                FacetComparison::Identity => Arc::ptr_eq(&output, old_output),
                FacetComparison::Inputs => same_inputs(&values, old_inputs, None)?,
                FacetComparison::Custom(compare) => compare(&output, old_output)?,
            },
        };
        let output = if equal { old_output.clone() } else { output };
        Ok((
            Cached::facet(output, values, &definition.compare_output),
            !equal,
        ))
    }
}

fn same_inputs(
    next: &[Value],
    previous: &[Value],
    compare: Option<&CompareValue>,
) -> Result<bool, ConfigurationError> {
    if next.len() != previous.len() {
        return Ok(false);
    }
    for (next, old) in next.iter().zip(previous) {
        let equal = match compare {
            Some(compare) => compare(next, old)?,
            None => Arc::ptr_eq(next, old),
        };
        if !equal {
            return Ok(false);
        }
    }
    Ok(true)
}

enum Slot {
    Field(Arc<FieldDefinition>),
    Input(Arc<FacetInput>),
    Facet(Vec<Arc<FacetInput>>),
}

fn is_dynamic(providers: &[Arc<FacetInput>]) -> bool {
    providers
        .iter()
        .any(|input| matches!(input.source, InputSource::Dynamic { .. }))
}

struct Configuration {
    root: Extension,
    compartments: HashMap<Key, Extension>,
    slots: HashMap<Key, Slot>,
    order: Vec<Key>,
    static_count: usize,
}

impl Configuration {
    fn new(
        root: Extension,
        overrides: &HashMap<Key, Extension>,
    ) -> Result<Self, ConfigurationError> {
        let mut builder = Flatten {
            overrides,
            compartments: HashMap::new(),
            seen: HashMap::new(),
            leaves: Vec::new(),
        };
        builder.visit(&root, Precedence::Default)?;
        builder.leaves.sort_by_key(|(precedence, _)| *precedence);
        let mut slots = HashMap::new();
        let mut fields = Vec::new();
        let mut facets = BTreeMap::<Key, Vec<_>>::new();
        for (priority, extension) in builder.leaves {
            if builder.seen[&extension.0.key] != priority {
                continue;
            }
            match &extension.0.kind {
                ExtensionKind::Field(field) => {
                    slots.insert(field.key, Slot::Field(field.clone()));
                    fields.push(field.key);
                }
                ExtensionKind::Facet(input) => {
                    facets
                        .entry(input.definition.key)
                        .or_default()
                        .push(input.clone());
                }
                _ => unreachable!("only field and facet definitions are leaves"),
            }
        }
        // Static facets are resolved before fields. Then resolve dynamic facets
        // in definition order, with each computed input before its output.
        let mut order = Vec::new();
        let mut dynamic = Vec::new();
        for (key, providers) in facets {
            if is_dynamic(&providers) {
                for input in &providers {
                    if matches!(input.source, InputSource::Dynamic { .. }) {
                        slots.insert(input.key, Slot::Input(input.clone()));
                        dynamic.push(input.key);
                    }
                }
                dynamic.push(key);
            } else {
                order.push(key);
            }
            slots.insert(key, Slot::Facet(providers));
        }
        let static_count = order.len();
        order.extend(fields);
        order.extend(dynamic);
        let compartments = builder
            .compartments
            .into_iter()
            .map(|(key, (_, extension))| (key, extension))
            .collect();
        Ok(Self {
            root,
            compartments,
            slots,
            order,
            static_count,
        })
    }
}

struct Flatten<'a> {
    overrides: &'a HashMap<Key, Extension>,
    compartments: HashMap<Key, (Key, Extension)>,
    seen: HashMap<Key, Precedence>,
    leaves: Vec<(Precedence, Extension)>,
}

impl Flatten<'_> {
    fn visit(
        &mut self,
        extension: &Extension,
        priority: Precedence,
    ) -> Result<(), ConfigurationError> {
        let node = &extension.0;
        if self
            .seen
            .get(&node.key)
            .is_some_and(|&known| known <= priority)
        {
            return Ok(());
        }
        self.seen.insert(node.key, priority);
        match &node.kind {
            ExtensionKind::Group(group) => {
                for extension in group {
                    self.visit(extension, priority)?;
                }
            }
            ExtensionKind::Priority(priority, content) => self.visit(content, *priority)?,
            ExtensionKind::Compartment(key, content) => {
                if self
                    .compartments
                    .get(key)
                    .is_some_and(|(owner, _)| *owner != node.key)
                {
                    return Err(ConfigurationError::DuplicateCompartment);
                }
                let content = self.overrides.get(key).unwrap_or(content);
                self.compartments.insert(*key, (node.key, content.clone()));
                self.visit(content, priority)?;
            }
            ExtensionKind::Field(field) => {
                self.leaves.push((priority, extension.clone()));
                if let Some(provided) = &field.provides {
                    self.visit(provided, priority)?;
                }
            }
            ExtensionKind::Facet(input) => {
                self.leaves.push((priority, extension.clone()));
                if let Some(enabled) = &input.definition.enables {
                    self.visit(enabled, Precedence::Default)?;
                }
            }
        }
        Ok(())
    }
}
