use std::{borrow::Cow, fmt, ops::Range, sync::Arc};

use regress::{Flags, Match, Regex};

use self::captures::NamedCapture;

mod captures;
mod compat;

/// A stateful ECMAScript-style regular expression for use with a UTF-16 stream.
///
/// `g` and `y` use [`Self::last_index`]. Unicode modes (`u` and `v`) match code
/// points, including lone surrogates; other modes match individual code units.
/// All indices and ranges are measured in code units.
///
/// Matching and most syntax rules come from `regress`. The wrapper
/// adapts surrogate escapes, Unicode ignore-case word escapes, and some
/// legacy (Annex B) escapes. It resolves named backreferences and permits
/// duplicate names only in disjoint alternatives. This is not a full JavaScript
/// engine. Backreferences honor the case flag at each reference site. Some
/// non-Unicode case-insensitive reference comparisons can still differ.
/// Negated Unicode sets reject expressions that may contain strings, including
/// through nested classes. The backend can reject a string property directly in
/// a valid negated intersection or subtraction. A positive class consisting
/// only of `\q` retains empty alternatives and tries longer strings first.
/// Standalone singleton strings follow pinned V8 case folding. Small positive
/// case-sensitive expressions of literal sets and `\q` also use these rules.
/// Mixed or nested case-folded, complemented, or property sets can differ.
/// A simple ASCII letter range in a short legacy pattern folds only within
/// ASCII. Other legacy classes and Unicode set operations can differ under
/// ignore-case matching. Unicode matches start at
/// code point boundaries. Node can match an empty string inside a surrogate
/// pair, for example with `/\B/gu` on `x🦀a`.
/// Use trusted mode patterns: the engine backtracks.
#[derive(Clone, Debug)]
pub struct RegExp {
    regex: Regex,
    names: Arc<[NamedCapture]>,
    global: bool,
    sticky: bool,
    unicode: bool,
    has_indices: bool,
    pub last_index: isize,
}

#[derive(Debug)]
pub enum RegExpError {
    /// A flag is unknown or repeated, or `u` and `v` were both set.
    InvalidFlags,
    /// The pattern has invalid syntax or is not supported by the backend.
    Pattern(regress::Error),
}

impl fmt::Display for RegExpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidFlags => f.write_str("invalid ECMAScript regular expression flags"),
            Self::Pattern(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for RegExpError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidFlags => None,
            Self::Pattern(error) => Some(error),
        }
    }
}

impl RegExp {
    /// Compile a UTF-8 pattern. Pass flags separately, without slash delimiters.
    pub fn new(pattern: &str, flags: &str) -> Result<Self, RegExpError> {
        Self::new_utf16(&pattern.encode_utf16().collect::<Vec<_>>(), flags)
    }

    /// Compile a pattern that can contain lone UTF-16 surrogates.
    pub fn new_utf16(pattern: &[u16], flags: &str) -> Result<Self, RegExpError> {
        let mut seen = 0;
        for flag in flags.bytes() {
            let bit = b"dgimsuvy"
                .iter()
                .position(|&valid| valid == flag)
                .map(|index| 1 << index)
                .ok_or(RegExpError::InvalidFlags)?;
            if seen & bit != 0 {
                return Err(RegExpError::InvalidFlags);
            }
            seen |= bit;
        }
        let mut engine = Flags::from(flags);
        if engine.unicode_sets {
            if engine.unicode {
                return Err(RegExpError::InvalidFlags);
            }
            engine.unicode = true;
        }
        let pattern = if engine.unicode {
            compat::unicode_escapes(pattern).map_err(RegExpError::Pattern)?
        } else {
            Cow::Borrowed(pattern)
        };
        let captures::CapturePattern { source, names } =
            captures::resolve(&pattern, engine).map_err(RegExpError::Pattern)?;
        let regex = if engine.unicode {
            let validated = compile_unicode(&source, engine).map_err(RegExpError::Pattern)?;
            let compatible = compat::unicode_pattern(&source, engine.unicode_sets, engine.icase)
                .map_err(RegExpError::Pattern)?;
            if let Cow::Owned(source) = compatible {
                compile_unicode(&source, engine).map_err(RegExpError::Pattern)?
            } else {
                validated
            }
        } else {
            let source =
                compat::legacy_pattern(&source, engine.icase).map_err(RegExpError::Pattern)?;
            Regex::from_unicode(source.iter().copied().map(u32::from), engine)
                .map_err(RegExpError::Pattern)?
        };
        Ok(Self {
            regex,
            names: names.into(),
            global: flags.contains('g'),
            sticky: flags.contains('y'),
            unicode: engine.unicode,
            has_indices: flags.contains('d'),
            last_index: 0,
        })
    }

    /// Run `RegExp.test` on the given input and update `last_index` if stateful.
    pub fn test(&mut self, input: &[u16]) -> bool {
        self.exec(input).is_some()
    }

    fn exec(&mut self, input: &[u16]) -> Option<Match> {
        let stateful = self.global || self.sticky;
        let mut start = if stateful {
            self.last_index.max(0) as usize
        } else {
            0
        };
        if self.unicode
            && start > 0
            && start < input.len()
            && is_surrogate_pair(&input[start - 1..])
        {
            // The UTF-16 offset points inside a code point. Start on its lead unit.
            start -= 1;
        }
        let matched = if start > input.len() {
            None
        } else if self.unicode {
            self.regex.find_from_utf16(input, start).next()
        } else {
            self.regex.find_from_ucs2(input, start).next()
        }
        .filter(|matched| !self.sticky || matched.start() == start);
        if stateful {
            self.last_index = matched.as_ref().map_or(0, |matched| matched.end() as isize);
        }
        matched
    }

    pub(super) fn match_string<'line>(
        &mut self,
        input: &'line [u16],
    ) -> Option<RegExpMatch<'line>> {
        if !self.global {
            return Some(RegExpMatch::Single(RegExpCaptures {
                input,
                matched: self.exec(input)?,
                names: Arc::clone(&self.names),
                has_indices: self.has_indices,
            }));
        }

        self.last_index = 0;
        let mut matches = Vec::new();
        while let Some(matched) = self.exec(input) {
            let range = matched.range();
            let text = &input[range.clone()];
            matches.push(text);
            if text.is_empty() {
                if range.end == input.len() {
                    self.last_index = 0;
                    break;
                }
                // Empty global matches step by one code point in Unicode modes.
                self.last_index += if self.unicode && is_surrogate_pair(&input[range.end..]) {
                    2
                } else {
                    1
                };
            }
        }
        (!matches.is_empty()).then_some(RegExpMatch::Global(matches))
    }
}

fn compile_unicode(pattern: &[u16], flags: Flags) -> Result<Regex, regress::Error> {
    let codepoints = char::decode_utf16(pattern.iter().copied()).map(|result| match result {
        Ok(ch) => u32::from(ch),
        Err(error) => u32::from(error.unpaired_surrogate()),
    });
    Regex::from_unicode(codepoints, flags)
}

fn is_surrogate_pair(units: &[u16]) -> bool {
    matches!(units, [0xd800..=0xdbff, 0xdc00..=0xdfff, ..])
}

/// The two result shapes of JavaScript's `String.match`.
#[derive(Debug)]
pub enum RegExpMatch<'line> {
    Single(RegExpCaptures<'line>),
    /// With `g`, only the full text of each match is returned. There is no
    /// match index, input property, or capture data, even with the `d` flag.
    Global(Vec<&'line [u16]>),
}

impl<'line> RegExpMatch<'line> {
    pub(super) fn first(&self) -> &'line [u16] {
        match self {
            Self::Single(captures) => &captures.input[captures.matched.range()],
            Self::Global(matches) => matches[0],
        }
    }
}

/// A non-global match, including its input suffix and captures.
#[derive(Debug)]
pub struct RegExpCaptures<'line> {
    input: &'line [u16],
    matched: Match,
    names: Arc<[NamedCapture]>,
    has_indices: bool,
}

impl<'line> RegExpCaptures<'line> {
    /// The start of the match, relative to the input suffix.
    pub fn index(&self) -> usize {
        self.matched.start()
    }

    pub fn input(&self) -> &'line [u16] {
        self.input
    }

    /// Index zero is the full match. Unmatched groups have no value.
    pub fn get(&self, index: usize) -> Option<&'line [u16]> {
        self.matched.group(index).map(|range| &self.input[range])
    }

    /// Iterate over the full match (index zero) and all numbered captures.
    pub fn captures(&self) -> impl ExactSizeIterator<Item = Option<&'line [u16]>> + '_ {
        self.matched
            .groups()
            .map(|range| range.map(|range| &self.input[range]))
    }

    /// Iterate over named captures, including unmatched ones.
    pub fn named_captures(
        &self,
    ) -> impl ExactSizeIterator<Item = (&str, Option<&'line [u16]>)> + '_ {
        self.named_ranges()
            .map(|(name, range)| (name, range.map(|range| &self.input[range])))
    }

    /// With `d`, return a range for the match and each participating capture.
    pub fn indices(&self) -> Option<impl ExactSizeIterator<Item = Option<Range<usize>>> + '_> {
        self.has_indices.then(|| self.matched.groups())
    }

    /// With `d`, return the named capture ranges relative to the input suffix.
    pub fn named_indices(
        &self,
    ) -> Option<impl ExactSizeIterator<Item = (&str, Option<Range<usize>>)> + '_> {
        (self.has_indices && !self.names.is_empty()).then(|| self.named_ranges())
    }

    fn named_ranges(&self) -> impl ExactSizeIterator<Item = (&str, Option<Range<usize>>)> + '_ {
        self.names.iter().map(|capture| {
            let range = capture
                .numbers
                .iter()
                .find_map(|&number| self.matched.group(number));
            (capture.name.as_str(), range)
        })
    }
}
