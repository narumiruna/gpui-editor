//! State-configured view plugins from CodeMirror view 6.44.0.

use std::{
    any::Any,
    sync::{Arc, LazyLock},
};

use crate::{EditorState, Extension, Facet};

use super::{ViewException, ViewPlugin, ViewUpdate, log_exception};

type Listener = Arc<dyn Fn(&ViewUpdate<'_>) -> Result<(), ViewException> + Send + Sync>;
static UPDATE_LISTENERS: LazyLock<Facet<Listener, Vec<Listener>>> = LazyLock::new(Facet::collect);

/// Observe accepted updates after installed plugins. A listener error goes to
/// the exception sink. It does not disable the listener or roll back the edit.
pub fn update_listener(
    listener: impl Fn(&ViewUpdate<'_>) -> Result<(), ViewException> + Send + Sync + 'static,
) -> Extension {
    UPDATE_LISTENERS.of(Arc::new(listener))
}

pub(super) fn has_listeners(state: &EditorState) -> bool {
    !state
        .context()
        .facet(&UPDATE_LISTENERS)
        .expect("typed update listener facet")
        .is_empty()
}

pub(super) fn notify_listeners(update: &ViewUpdate<'_>) {
    for listener in update
        .state
        .context()
        .facet(&UPDATE_LISTENERS)
        .expect("typed update listener facet")
        .iter()
    {
        if let Err(error) = listener(update) {
            log_exception(update.state, &error.with_context("view update listener"));
        }
    }
}

type Factory<A, P> = dyn Fn(&EditorState, &A) -> Result<P, ViewException> + Send + Sync;
type Create = dyn Fn(&EditorState) -> Result<Box<dyn BoundPlugin>, ViewException> + Send + Sync;

trait BoundPlugin {
    fn value(&self) -> &dyn Any;
    fn update(&mut self, update: &ViewUpdate<'_>) -> Result<(), ViewException>;
    fn destroy(&mut self);
}

impl<P: ViewPlugin + 'static> BoundPlugin for P {
    fn value(&self) -> &dyn Any {
        self
    }
    fn update(&mut self, update: &ViewUpdate<'_>) -> Result<(), ViewException> {
        self.try_update(update)
    }
    fn destroy(&mut self) {
        ViewPlugin::destroy(self);
    }
}

/// A typed plugin definition shared by editor states. The view owns each
/// constructed instance; no mutable plugin state is shared between views.
pub struct ViewPluginType<A, P> {
    identity: Arc<()>,
    create: Arc<Factory<A, P>>,
}

impl<A, P> Clone for ViewPluginType<A, P> {
    fn clone(&self) -> Self {
        Self {
            identity: self.identity.clone(),
            create: self.create.clone(),
        }
    }
}

impl<A: Send + Sync + 'static, P: ViewPlugin + 'static> ViewPluginType<A, P> {
    pub fn define(
        create: impl Fn(&EditorState, &A) -> Result<P, ViewException> + Send + Sync + 'static,
    ) -> Self {
        Self {
            identity: Arc::new(()),
            create: Arc::new(create),
        }
    }

    /// Bind the constructor argument. When the same definition occurs more
    /// than once, the highest-priority binding wins. Keep this extension to
    /// preserve the instance across reconfiguration. A fresh `of` creates
    /// a fresh binding, even if its argument compares equal.
    pub fn of(&self, argument: A) -> Extension {
        let create = self.create.clone();
        PLUGINS.of(Arc::new(Spec {
            identity: self.identity.clone(),
            create: Box::new(move |state| Ok(Box::new(create(state, &argument)?))),
        }))
    }
}

struct Spec {
    identity: Arc<()>,
    create: Box<Create>,
}

type Specs = Vec<Arc<Spec>>;
static PLUGINS: LazyLock<Facet<Arc<Spec>, Specs>> = LazyLock::new(|| {
    Facet::define(
        |inputs: &[&Arc<Spec>]| {
            let mut specs: Specs = Vec::new();
            for &input in inputs {
                if !specs
                    .iter()
                    .any(|spec| Arc::ptr_eq(&spec.identity, &input.identity))
                {
                    specs.push(input.clone());
                }
            }
            specs
        },
        |options| {
            options.compare(|a, b| {
                a.len() == b.len() && a.iter().zip(b).all(|(a, b)| Arc::ptr_eq(a, b))
            })
        },
    )
});

#[derive(Default)]
pub(super) struct ConfiguredPlugins {
    entries: Vec<Entry>,
}

struct Entry {
    spec: Arc<Spec>,
    instance: Instance,
}

enum Instance {
    New,
    Live(Box<dyn BoundPlugin>),
    Disabled,
}

impl ConfiguredPlugins {
    pub(super) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub(super) fn get<A, P: ViewPlugin + 'static>(
        &self,
        kind: &ViewPluginType<A, P>,
    ) -> Option<&P> {
        let entry = self
            .entries
            .iter()
            .find(|entry| Arc::ptr_eq(&entry.spec.identity, &kind.identity))?;
        let Instance::Live(plugin) = &entry.instance else {
            return None;
        };
        plugin.value().downcast_ref()
    }

    pub(super) fn refresh(&mut self, state: &EditorState, update: Option<&ViewUpdate<'_>>) {
        let specs = state
            .context()
            .facet(&PLUGINS)
            .expect("typed view plugin facet");
        if !(self.entries.len() == specs.len()
            && self
                .entries
                .iter()
                .zip(specs.iter())
                .all(|(entry, spec)| Arc::ptr_eq(&entry.spec, spec)))
        {
            let mut old = std::mem::take(&mut self.entries);
            self.entries.extend(specs.iter().map(|spec| {
                if let Some(i) = old.iter().position(|entry| Arc::ptr_eq(&entry.spec, spec)) {
                    old.remove(i)
                } else {
                    Entry {
                        spec: spec.clone(),
                        instance: Instance::New,
                    }
                }
            }));
            for mut removed in old {
                removed.destroy();
            }
        }
        for entry in &mut self.entries {
            match &mut entry.instance {
                Instance::New => {
                    entry.instance = match (entry.spec.create)(state) {
                        Ok(plugin) => Instance::Live(plugin),
                        Err(error) => {
                            log_exception(state, &error.with_context("view plugin construction"));
                            Instance::Disabled
                        }
                    };
                }
                Instance::Live(plugin) => {
                    if let Some(update) = update
                        && let Err(error) = plugin.update(update)
                    {
                        log_exception(state, &error.with_context("view plugin update"));
                        entry.destroy();
                    }
                }
                Instance::Disabled => {}
            }
        }
    }

    pub(super) fn clear(&mut self) {
        for mut entry in self.entries.drain(..) {
            entry.destroy();
        }
    }
}

impl Entry {
    fn destroy(&mut self) {
        if let Instance::Live(mut plugin) =
            std::mem::replace(&mut self.instance, Instance::Disabled)
        {
            plugin.destroy();
        }
    }
}

impl Drop for ConfiguredPlugins {
    fn drop(&mut self) {
        self.clear();
    }
}
