//! Native key bindings and scoped dispatch based on CodeMirror view 6.44.0.
//! See `docs/licenses/codemirror-view.txt`.

use std::{collections::HashMap, error::Error, fmt, time::Duration};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeyPlatform {
    Mac,
    Windows,
    Linux,
    Other,
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct KeyModifiers {
    pub alt: bool,
    pub control: bool,
    pub meta: bool,
    pub shift: bool,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct Stroke {
    name: String,
    modifiers: KeyModifiers,
}

impl Stroke {
    fn parse(mut spec: &str, platform: KeyPlatform) -> Result<Self, KeymapError> {
        let mut modifiers = KeyModifiers::default();
        while let Some((modifier, rest)) = spec.split_once('-').filter(|(_, rest)| !rest.is_empty())
        {
            match modifier.to_ascii_lowercase().as_str() {
                "a" | "alt" => modifiers.alt = true,
                "c" | "ctrl" | "control" => modifiers.control = true,
                "m" | "meta" | "cmd" => modifiers.meta = true,
                "s" | "shift" => modifiers.shift = true,
                "mod" if platform == KeyPlatform::Mac => modifiers.meta = true,
                "mod" => modifiers.control = true,
                _ => return Err(KeymapError::UnknownModifier(modifier.to_owned())),
            }
            spec = rest;
        }
        if spec.is_empty() {
            return Err(KeymapError::EmptyStroke);
        }
        Ok(Self {
            name: if spec == "Space" { " " } else { spec }.to_owned(),
            modifiers,
        })
    }
}

/// One native key-down event. Names use the CodeMirror notation (`ArrowLeft`, `Enter`, `a`).
/// Supply layout-independent names when the host can report them.
#[derive(Clone, Copy, Debug)]
pub struct KeyEvent<'a> {
    pub name: &'a str,
    pub modifiers: KeyModifiers,
    pub base_name: Option<&'a str>,
    pub shifted_base_name: Option<&'a str>,
    pub modifier_only: bool,
}

impl<'a> KeyEvent<'a> {
    pub fn new(name: &'a str, modifiers: KeyModifiers) -> Self {
        Self {
            name,
            modifiers,
            base_name: None,
            shifted_base_name: None,
            modifier_only: matches!(name, "Alt" | "Control" | "Meta" | "Shift"),
        }
    }

    fn strokes(self, platform: KeyPlatform) -> impl Iterator<Item = Stroke> {
        let character = self.name != " " && self.name.chars().count() == 1;
        let m = self.modifiers;
        let allow_base = character
            && (m.alt || m.control || m.meta)
            && !(platform == KeyPlatform::Windows && m.control && m.alt)
            && !(platform == KeyPlatform::Mac && m.alt && !(m.control || m.meta));
        let base = self
            .base_name
            .filter(|name| allow_base && *name != self.name);
        let second = base
            .map(|name| (name, m.shift))
            .or_else(|| (character && m.shift).then_some((self.name, true)));
        let third = base.and_then(|base| {
            self.shifted_base_name
                .filter(|name| m.shift && *name != self.name && *name != base)
                .map(|name| (name, false))
        });
        [Some((self.name, m.shift && !character)), second, third]
            .into_iter()
            .flatten()
            .map(move |(name, shift)| Stroke {
                name: name.to_owned(),
                modifiers: KeyModifiers { shift, ..m },
            })
    }
}

/// The caller supplies stable command identifiers and runs the matching commands.
/// Higher priority bindings must come first.
#[derive(Clone, Debug)]
pub struct KeyBinding<C> {
    pub key: Option<String>,
    pub mac: Option<String>,
    pub windows: Option<String>,
    pub linux: Option<String>,
    pub scope: String,
    pub run: Option<C>,
    pub shift: Option<C>,
    pub any: Option<C>,
    pub prevent_default: bool,
    pub stop_propagation: bool,
}

impl<C> Default for KeyBinding<C> {
    fn default() -> Self {
        Self {
            key: None,
            mac: None,
            windows: None,
            linux: None,
            scope: "editor".into(),
            run: None,
            shift: None,
            any: None,
            prevent_default: false,
            stop_propagation: false,
        }
    }
}

impl<C> KeyBinding<C> {
    pub fn new(key: impl Into<String>, command: C) -> Self {
        Self {
            key: Some(key.into()),
            run: Some(command),
            ..Self::default()
        }
    }

    fn key_for(&self, platform: KeyPlatform) -> Option<&str> {
        match platform {
            KeyPlatform::Mac => self.mac.as_deref(),
            KeyPlatform::Windows => self.windows.as_deref(),
            KeyPlatform::Linux => self.linux.as_deref(),
            KeyPlatform::Other => None,
        }
        .filter(|key| !key.is_empty())
        .or(self.key.as_deref())
        .filter(|key| !key.is_empty())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum KeymapError {
    EmptyStroke,
    UnknownModifier(String),
    ConflictingPrefix(String),
}

impl fmt::Display for KeymapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyStroke => f.write_str("key binding has an empty stroke"),
            Self::UnknownModifier(name) => write!(f, "unrecognized key modifier: {name}"),
            Self::ConflictingPrefix(spec) => {
                write!(f, "key binding is both a command and a prefix: {spec}")
            }
        }
    }
}

impl Error for KeymapError {}

#[derive(Clone, Debug)]
enum Handler<C> {
    Prefix,
    Command(C),
    Any { command: C, id: usize },
}

#[derive(Debug)]
struct Binding<C> {
    handlers: Vec<Handler<C>>,
    prevent_default: bool,
    stop_propagation: bool,
}

impl<C> Default for Binding<C> {
    fn default() -> Self {
        Self {
            handlers: Vec::new(),
            prevent_default: false,
            stop_propagation: false,
        }
    }
}

#[derive(Debug)]
struct Scope<C> {
    keys: HashMap<Vec<Stroke>, Binding<C>>,
    any: Binding<C>,
}

impl<C> Default for Scope<C> {
    fn default() -> Self {
        Self {
            keys: HashMap::new(),
            any: Binding::default(),
        }
    }
}

/// In-progress key sequence for one view. Reuse it between events and clear it on blur.
#[derive(Debug, Default)]
pub struct KeySequence {
    prefix: Option<(String, Vec<Stroke>, Duration)>,
}

impl KeySequence {
    pub fn clear(&mut self) {
        self.prefix = None;
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct KeyResult {
    pub handled: bool,
    pub stop_propagation: bool,
}

#[derive(Debug)]
pub struct Keymap<C> {
    platform: KeyPlatform,
    scopes: HashMap<String, Scope<C>>,
}

impl<C: Clone + Eq> Keymap<C> {
    pub fn new(
        platform: KeyPlatform,
        bindings: impl IntoIterator<Item = KeyBinding<C>>,
    ) -> Result<Self, KeymapError> {
        let mut scopes: HashMap<String, Scope<C>> = HashMap::new();
        let mut prefixes = HashMap::new();
        let mut next_any = 0;
        for binding in bindings {
            let spec = binding.key_for(platform);
            for scope in binding.scope.split_whitespace() {
                let scope = scopes.entry(scope.to_owned()).or_default();
                if let Some(any) = &binding.any {
                    for target in scope
                        .keys
                        .values_mut()
                        .chain(std::iter::once(&mut scope.any))
                    {
                        target.handlers.push(Handler::Any {
                            command: any.clone(),
                            id: next_any,
                        });
                        next_any += 1;
                    }
                }
                let Some(spec) = spec else { continue };
                for (spec, command) in std::iter::once((spec.to_owned(), binding.run.as_ref()))
                    .chain(
                        binding
                            .shift
                            .as_ref()
                            .map(|run| (format!("Shift-{spec}"), Some(run))),
                    )
                {
                    // A literal trailing space is part of the last key name, as in the reference.
                    let mut remaining = spec.as_str();
                    let mut strokes = Vec::new();
                    while let Some((stroke, rest)) = remaining
                        .split_once(' ')
                        .filter(|(_, rest)| !rest.is_empty())
                    {
                        strokes.push(Stroke::parse(stroke, platform)?);
                        remaining = rest;
                    }
                    strokes.push(Stroke::parse(remaining, platform)?);
                    for length in 1..=strokes.len() {
                        let is_prefix = length < strokes.len();
                        let prefix = &strokes[..length];
                        match prefixes.get(prefix) {
                            Some(previous) if *previous != is_prefix => {
                                return Err(KeymapError::ConflictingPrefix(spec));
                            }
                            None => {
                                prefixes.insert(prefix.to_vec(), is_prefix);
                            }
                            _ => {}
                        }
                        let entry = scope
                            .keys
                            .entry(prefix.to_vec())
                            .or_insert_with(|| Binding {
                                handlers: if is_prefix {
                                    vec![Handler::Prefix]
                                } else {
                                    scope.any.handlers.clone()
                                },
                                prevent_default: is_prefix,
                                stop_propagation: false,
                            });
                        if !is_prefix {
                            if let Some(command) = command {
                                entry.handlers.push(Handler::Command(command.clone()));
                            }
                            entry.prevent_default |= binding.prevent_default;
                            entry.stop_propagation |= binding.stop_propagation;
                        }
                    }
                }
            }
        }
        Ok(Self { platform, scopes })
    }

    /// Dispatch against the named scope. `now` must come from the same monotonic clock
    /// for all calls to this sequence. A false callback result falls through.
    pub fn dispatch<'a>(
        &'a self,
        scope: &str,
        event: KeyEvent<'_>,
        now: Duration,
        sequence: &mut KeySequence,
        mut command: impl FnMut(&C) -> bool,
    ) -> KeyResult {
        let mut result = KeyResult::default();
        if sequence
            .prefix
            .as_ref()
            .is_some_and(|(_, _, deadline)| now >= *deadline)
        {
            sequence.clear();
        }
        let mut strokes = match &sequence.prefix {
            Some((active_scope, strokes, _)) if active_scope == scope => {
                let strokes = strokes.clone();
                if !event.modifier_only {
                    result.handled = true;
                    sequence.clear();
                }
                strokes
            }
            _ => Vec::new(),
        };
        let Some(bindings) = self.scopes.get(scope) else {
            return result;
        };
        let mut ran = Vec::new();
        let mut ran_any = Vec::new();
        let mut run = |binding: &'a Binding<C>, strokes: &[Stroke]| {
            for handler in &binding.handlers {
                let accepted = match handler {
                    Handler::Prefix => {
                        sequence.prefix = Some((
                            scope.to_owned(),
                            strokes.to_vec(),
                            now.saturating_add(Duration::from_secs(4)),
                        ));
                        true
                    }
                    Handler::Command(id) if !ran.contains(&id) => {
                        ran.push(id);
                        command(id)
                    }
                    Handler::Any {
                        command: id,
                        id: any,
                    } if !ran_any.contains(any) => {
                        ran_any.push(*any);
                        command(id)
                    }
                    Handler::Command(_) | Handler::Any { .. } => false,
                };
                if accepted {
                    result = KeyResult {
                        handled: true,
                        stop_propagation: result.stop_propagation || binding.stop_propagation,
                    };
                    return true;
                }
            }
            if binding.prevent_default {
                result.handled = true;
                result.stop_propagation |= binding.stop_propagation;
            }
            false
        };
        for stroke in event.strokes(self.platform) {
            strokes.push(stroke);
            if bindings
                .keys
                .get(&strokes)
                .is_some_and(|binding| run(binding, &strokes))
            {
                return result;
            }
            strokes.pop();
        }
        run(&bindings.any, &strokes);
        result
    }
}
