//! Native `simpleMode` factory from `@codemirror/legacy-modes` 6.5.5.
//! See `docs/licenses/codemirror-legacy-modes.txt` for the upstream MIT notice.
//!
//! Patterns use `regress` ECMAScript syntax and retain only the ignore-case
//! and Unicode options, as upstream does. They always read Unicode scalars
//! from UTF-8, even without the Unicode option. Thus patterns cannot match
//! part of a surrogate pair, and length-sensitive matches can differ from
//! JavaScript's non-Unicode mode. Engine differences include some legacy
//! escapes (such as `\x` without two hex digits), Unicode ignore-case word
//! classes, and duplicate names in disjoint alternatives. Use trusted
//! patterns: the engine can backtrack.
//!
//! A rule that matches no text must change the state or stack. Other empty
//! matches are skipped so a rule cannot stall the stream at a line separator.
//! Copied states have independent pending tokens.

use std::{
    borrow::Cow,
    collections::{HashMap, VecDeque},
    fmt,
};

use regress::{Flags, Match, Regex};

use crate::{
    language::{LanguageData, StreamLanguage},
    stream_parser::{StreamParser, StringStream},
};

mod bundled;

pub(super) use bundled::{bundled, data};

/// Metadata and parser options for [`simple_mode`].
pub struct SimpleModeOptions {
    pub name: Option<String>,
    pub dont_indent_states: Vec<String>,
    pub merge_tokens: bool,
    /// Document metadata. The other options affect the parser and are not
    /// automatically added as keys to this data.
    pub language_data: LanguageData,
}

impl Default for SimpleModeOptions {
    fn default() -> Self {
        Self {
            name: None,
            dont_indent_states: Vec::new(),
            merge_tokens: true,
            language_data: LanguageData::default(),
        }
    }
}

/// A rule tried in order in its state. The pattern is anchored to the cursor.
#[derive(Default)]
pub struct SimpleRule {
    /// The pattern source, without slash delimiters. The default is empty.
    pub regex: String,
    pub ignore_case: bool,
    pub unicode: bool,
    pub token: SimpleToken,
    pub sol: bool,
    /// Go to this state without changing the state stack.
    pub next: Option<String>,
    /// Save the current state, then go to this state.
    pub push: Option<String>,
    /// Restore the last saved state, if one exists.
    pub pop: bool,
    pub indent: bool,
    pub dedent: bool,
    /// When `Some(false)`, do not count this dedent in indentation queries.
    pub dedent_if_line_start: Option<bool>,
}

impl SimpleRule {
    pub fn new(regex: impl Into<String>) -> Self {
        Self {
            regex: regex.into(),
            ..Self::default()
        }
    }
}

/// A fixed or computed style. Fixed styles replace dots with spaces.
#[derive(Default)]
pub enum SimpleToken {
    #[default]
    None,
    Style(String),
    /// Styles for the numbered captures, in order. Use `None` for unstyled text.
    /// With two or more capture groups, the groups should partition the match
    /// into adjacent spans. Missing and empty captures emit no tokens.
    Groups(Vec<Option<String>>),
    /// Compute the style from a match. With two or more captures, only the
    /// first capture gets a style: the first item of the result, or the first
    /// scalar if it is a string. Later captures are unstyled, as upstream.
    Callback(Box<dyn Fn(SimpleMatch<'_>) -> SimpleTokenValue + Send + Sync>),
}

/// A callback result. Styles from callbacks do not replace dots with spaces.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum SimpleTokenValue {
    #[default]
    None,
    Style(String),
    Groups(Vec<Option<String>>),
}

/// Captures passed to a token callback. Group zero is the full match.
#[derive(Clone, Copy)]
pub struct SimpleMatch<'a> {
    input: &'a str,
    matched: &'a Match,
}

impl<'a> SimpleMatch<'a> {
    /// The input suffix at the cursor before the rule matched.
    pub fn input(&self) -> &'a str {
        self.input
    }

    pub fn get(&self, index: usize) -> Option<&'a str> {
        self.matched
            .group(index)
            .and_then(|range| self.input.get(range))
    }

    pub fn named(&self, name: &str) -> Option<&'a str> {
        self.matched
            .named_group(name)
            .and_then(|range| self.input.get(range))
    }

    pub fn captures(&self) -> impl ExactSizeIterator<Item = Option<&'a str>> + '_ {
        self.matched
            .groups()
            .map(|range| range.and_then(|range| self.input.get(range)))
    }
}

/// A mode definition is invalid.
#[derive(Debug)]
pub enum SimpleModeError {
    MissingStart,
    DuplicateState(String),
    UnknownState(String),
    InvalidRegex {
        state: String,
        rule: usize,
        source: regress::Error,
    },
}

impl fmt::Display for SimpleModeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingStart => f.write_str("simple mode is missing the start state"),
            Self::DuplicateState(state) => write!(f, "duplicate simple mode state {state:?}"),
            Self::UnknownState(state) => write!(f, "unknown simple mode state {state:?}"),
            Self::InvalidRegex {
                state,
                rule,
                source,
            } => {
                write!(
                    f,
                    "invalid regex in simple mode state {state:?}, rule {rule}: {source}"
                )
            }
        }
    }
}

impl std::error::Error for SimpleModeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidRegex { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// A compiled rule set. Use it with a [`StreamParser`] consumer, or use
/// [`Self::into_language`] to include its language data in document services.
pub struct SimpleMode {
    start: usize,
    states: Vec<RuleSet>,
    has_indentation: bool,
    options: SimpleModeOptions,
}

struct RuleSet {
    rules: Vec<Rule>,
    dont_indent: bool,
}

struct Rule {
    regex: Regex,
    token: SimpleToken,
    transition: Transition,
    sol: bool,
    indent: bool,
    dedent: bool,
    scan_dedent: bool,
}

#[derive(Clone, Copy)]
enum Transition {
    Stay,
    Next(usize),
    Push(usize),
    Pop,
}

/// A snapshot of a [`SimpleMode`]. Use it only with the mode that created it.
#[derive(Clone, Debug)]
pub struct SimpleModeState {
    active: usize,
    stack: Vec<usize>,
    indent: Vec<usize>,
    pending: VecDeque<Pending>,
}

#[derive(Clone, Debug)]
struct Pending {
    bytes: usize,
    style: Option<String>,
}

/// Compile named rule lists. The `start` list is required.
/// When a rule has several transitions, `next` takes precedence over `push`,
/// which takes precedence over `pop`.
pub fn simple_mode(
    states: impl IntoIterator<Item = (impl Into<String>, Vec<SimpleRule>)>,
    options: SimpleModeOptions,
) -> Result<SimpleMode, SimpleModeError> {
    let mut lists = Vec::new();
    let mut names = HashMap::new();
    for (name, rules) in states {
        let name = name.into();
        if names.insert(name.clone(), lists.len()).is_some() {
            return Err(SimpleModeError::DuplicateState(name));
        }
        lists.push((name, rules));
    }
    let start = *names.get("start").ok_or(SimpleModeError::MissingStart)?;
    let lookup = |name: &str| {
        names
            .get(name)
            .copied()
            .ok_or_else(|| SimpleModeError::UnknownState(name.into()))
    };
    let mut has_indentation = false;
    let states = lists
        .into_iter()
        .map(|(name, rules)| {
            let dont_indent = options.dont_indent_states.contains(&name);
            let rules = rules
                .into_iter()
                .enumerate()
                .map(|(index, rule)| {
                    let transition = if let Some(next) = &rule.next {
                        Transition::Next(lookup(next)?)
                    } else if let Some(push) = &rule.push {
                        Transition::Push(lookup(push)?)
                    } else if rule.pop {
                        Transition::Pop
                    } else {
                        Transition::Stay
                    };
                    let regex = Regex::with_flags(
                        &format!("^(?:{})", rule.regex),
                        Flags {
                            icase: rule.ignore_case,
                            unicode: rule.unicode,
                            ..Flags::default()
                        },
                    )
                    .map_err(|source| SimpleModeError::InvalidRegex {
                        state: name.clone(),
                        rule: index,
                        source,
                    })?;
                    has_indentation |= rule.indent || rule.dedent;
                    Ok(Rule {
                        regex,
                        token: normalize(rule.token),
                        transition,
                        sol: rule.sol,
                        indent: rule.indent,
                        dedent: rule.dedent,
                        scan_dedent: rule.dedent && rule.dedent_if_line_start != Some(false),
                    })
                })
                .collect::<Result<_, SimpleModeError>>()?;
            Ok(RuleSet { rules, dont_indent })
        })
        .collect::<Result<_, SimpleModeError>>()?;
    Ok(SimpleMode {
        start,
        states,
        has_indentation,
        options,
    })
}

fn normalize(token: SimpleToken) -> SimpleToken {
    match token {
        SimpleToken::Style(style) if style.is_empty() => SimpleToken::None,
        SimpleToken::Style(style) => SimpleToken::Style(style.replace('.', " ")),
        SimpleToken::Groups(groups) => SimpleToken::Groups(
            groups
                .into_iter()
                .map(|style| style.map(|style| style.replace('.', " ")))
                .collect(),
        ),
        token => token,
    }
}

impl SimpleMode {
    pub fn language_data(&self) -> &LanguageData {
        &self.options.language_data
    }

    pub fn into_language(mut self) -> StreamLanguage<Self> {
        let data = std::mem::take(&mut self.options.language_data);
        StreamLanguage::new(self).with_language_data(data)
    }
}

impl StreamParser for SimpleMode {
    type State = SimpleModeState;

    fn name(&self) -> Option<&str> {
        self.options.name.as_deref()
    }

    fn start_state(&self, _indent_unit: usize) -> SimpleModeState {
        SimpleModeState {
            active: self.start,
            stack: Vec::new(),
            indent: Vec::new(),
            pending: VecDeque::new(),
        }
    }

    fn token(
        &self,
        stream: &mut StringStream<'_>,
        state: &mut SimpleModeState,
    ) -> Option<Cow<'_, str>> {
        if let Some(pending) = state.pending.pop_front() {
            advance(stream, pending.bytes);
            return pending.style.map(Cow::Owned);
        }
        let input = &stream.line()[stream.pos()..];
        for rule in &self.states[state.active].rules {
            if rule.sol && !stream.sol() {
                continue;
            }
            let Some(matched) = rule.regex.find(input) else {
                continue;
            };
            let before = (state.active, state.stack.len());
            match rule.transition {
                Transition::Stay => {}
                Transition::Next(next) => state.active = next,
                Transition::Push(next) => {
                    state.stack.push(state.active);
                    state.active = next;
                }
                Transition::Pop => {
                    if let Some(previous) = state.stack.pop() {
                        state.active = previous;
                    }
                }
            }
            if matched.end() == 0 && before == (state.active, state.stack.len()) {
                // An empty rule without a state change cannot read a token.
                continue;
            }
            if rule.indent {
                state
                    .indent
                    .push(stream.indentation().saturating_add(stream.indent_unit()));
            }
            if rule.dedent {
                state.indent.pop();
            }
            return rule.read_style(
                stream,
                state,
                SimpleMatch {
                    input,
                    matched: &matched,
                },
            );
        }
        stream.next();
        None
    }

    fn indent(
        &self,
        state: &SimpleModeState,
        mut text_after: &str,
        _indent_unit: usize,
    ) -> Option<usize> {
        let active = &self.states[state.active];
        if !self.has_indentation || active.dont_indent {
            return None;
        }
        let mut depth = state.indent.len();
        // Upstream scans repeated dedents in the current state's rules. It
        // does not apply a dedent rule's state transition in an indent query.
        while depth > 0 {
            let Some(bytes) = active
                .rules
                .iter()
                .filter(|rule| rule.scan_dedent)
                .find_map(|rule| {
                    rule.regex
                        .find(text_after)
                        .map(|matched| matched.end())
                        .filter(|&end| end != 0)
                })
            else {
                break;
            };
            depth -= 1;
            text_after = &text_after[bytes..];
        }
        Some(depth.checked_sub(1).map_or(0, |index| state.indent[index]))
    }

    fn merge_tokens(&self) -> bool {
        self.options.merge_tokens
    }
}

impl Rule {
    fn read_style<'a>(
        &'a self,
        stream: &mut StringStream<'_>,
        state: &mut SimpleModeState,
        matched: SimpleMatch<'_>,
    ) -> Option<Cow<'a, str>> {
        let split = matched.matched.captures.len() > 1;
        match &self.token {
            SimpleToken::Groups(groups) if split => read_groups(stream, state, matched, groups),
            // Upstream reads later styles from the callback itself, not its result.
            SimpleToken::Callback(_) if split => read_groups(stream, state, matched, &[]),
            _ => advance(stream, matched.matched.end()),
        }
        match &self.token {
            SimpleToken::None => None,
            SimpleToken::Style(style) => Some(Cow::Borrowed(style)),
            SimpleToken::Groups(groups) => {
                groups.first().and_then(Option::as_deref).map(Cow::Borrowed)
            }
            SimpleToken::Callback(callback) => match callback(matched) {
                SimpleTokenValue::None => None,
                SimpleTokenValue::Style(style) if split => {
                    style.chars().next().map(|ch| Cow::Owned(ch.to_string()))
                }
                SimpleTokenValue::Style(style) => Some(Cow::Owned(style)),
                SimpleTokenValue::Groups(groups) => {
                    groups.into_iter().next().flatten().map(Cow::Owned)
                }
            },
        }
    }
}

fn read_groups(
    stream: &mut StringStream<'_>,
    state: &mut SimpleModeState,
    matched: SimpleMatch<'_>,
    groups: &[Option<String>],
) {
    for (index, capture) in matched.captures().skip(2).enumerate() {
        if let Some(capture) = capture.filter(|capture| !capture.is_empty()) {
            state.pending.push_back(Pending {
                bytes: capture.len(),
                style: groups.get(index + 1).cloned().flatten(),
            });
        }
    }
    advance(stream, matched.get(1).map_or(0, str::len));
}

fn advance(stream: &mut StringStream<'_>, bytes: usize) {
    // Upstream consumes capture lengths in order, not the capture offsets.
    // Contiguous captures partitioned across tokens already end on boundaries.
    let line = stream.line();
    let mut pos = stream.pos().saturating_add(bytes).min(line.len());
    while !line.is_char_boundary(pos) {
        pos += 1;
    }
    stream.set_pos(pos).expect("a UTF-8 boundary");
}
