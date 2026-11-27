use std::{iter::Peekable, str::Chars};

use crate::search::SearchError;

const MAX_PATTERN_BYTES: usize = 384;
const MAX_DEPTH: usize = 12;
const MAX_REPEAT: u32 = 16;
const MAX_WORK: u32 = 768;
const MAX_CAPTURE_WORK: u32 = 96;

type Input<'a> = Peekable<Chars<'a>>;

/// Admit only patterns with bounded work per starting position. This backend
/// has no fuel counter, so unbounded quantifiers are not admissible, even in
/// a successful assertion. Bound both the number of paths and the work to
/// try all of them. The pattern length also bounds capture storage and the
/// size of class tests. There is no limit on the length of the editor text.
pub(super) fn compile(pattern: &str, ignore_case: bool) -> Result<regress::Regex, SearchError> {
    if pattern.len() > MAX_PATTERN_BYTES {
        return Err(too_complex());
    }
    let mut rest = pattern.chars().peekable();
    let mut syntax = Syntax::default();
    branch(&mut rest, 0, false, &mut syntax)?;
    if !syntax.advanced || syntax.max_numbered_ref > syntax.captures || rest.next().is_some() {
        return Err(invalid());
    }
    regress::Regex::with_flags(pattern, if ignore_case { "miu" } else { "mu" })
        .map_err(|error| SearchError::InvalidRegExp(error.to_string()))
}

#[derive(Default)]
struct Syntax {
    advanced: bool,
    captures: u32,
    max_numbered_ref: u32,
}

#[derive(Clone, Copy)]
struct Work {
    steps: u32,
    paths: u32,
}

impl Work {
    const EMPTY: Self = Self { steps: 0, paths: 1 };

    fn atom(steps: u32) -> Result<Self, SearchError> {
        Self::checked(steps, 1)
    }

    fn checked(steps: u32, paths: u32) -> Result<Self, SearchError> {
        if steps > MAX_WORK || paths > MAX_WORK {
            return Err(too_complex());
        }
        Ok(Self { steps, paths })
    }

    fn then(self, next: Self) -> Result<Self, SearchError> {
        Self::checked(
            self.steps + self.paths * next.steps,
            self.paths * next.paths,
        )
    }

    fn or(self, next: Self) -> Result<Self, SearchError> {
        Self::checked(self.steps + next.steps + 2, self.paths + next.paths)
    }

    fn repeat(self, min: u32, max: u32) -> Result<Self, SearchError> {
        let mut powers = Self::EMPTY;
        let mut total = Self { steps: 0, paths: 0 };
        for count in 0..=max {
            if count >= min {
                total = total.or(powers)?;
            }
            if count < max {
                powers = powers.then(self)?.then(Self::atom(2)?)?;
            }
        }
        Ok(total)
    }
}

fn branch(
    rest: &mut Input<'_>,
    depth: usize,
    in_capture: bool,
    syntax: &mut Syntax,
) -> Result<Work, SearchError> {
    if depth > MAX_DEPTH {
        return Err(too_complex());
    }
    let mut alternatives = None::<Work>;
    let mut current = Work::EMPTY;
    while let Some(ch) = rest.next_if(|ch| *ch != ')') {
        if ch == '|' {
            alternatives = Some(match alternatives {
                Some(branch) => branch.or(current)?,
                None => current,
            });
            current = Work::EMPTY;
            continue;
        }
        let mut assertion = false;
        let atom = match ch {
            '(' => {
                let capture = if rest.next_if_eq(&'?').is_none() {
                    true
                } else {
                    match rest.next() {
                        Some(':') => false,
                        Some('=' | '!') => {
                            syntax.advanced = true;
                            assertion = true;
                            false
                        }
                        Some('<') => {
                            if rest.next_if(|ch| matches!(ch, '=' | '!')).is_some() {
                                syntax.advanced = true;
                                assertion = true;
                                false
                            } else {
                                after_name(rest)?;
                                true
                            }
                        }
                        _ => return Err(invalid()),
                    }
                };
                syntax.captures += u32::from(capture);
                let body = branch(rest, depth + 1, in_capture || capture, syntax)?;
                if capture && body.steps > MAX_CAPTURE_WORK {
                    return Err(too_complex());
                }
                rest.next_if_eq(&')').ok_or_else(invalid)?;
                Work::atom(4)?.then(body)?.then(Work::atom(4)?)?
            }
            '[' => {
                let mut size = 1;
                while let Some(ch) = rest.next() {
                    if ch == ']' {
                        break;
                    }
                    if ch == '\\' {
                        match rest.next() {
                            Some('p' | 'P') => return Err(too_complex()),
                            Some('1'..='9') => return Err(invalid()),
                            Some('0') if rest.peek().is_some_and(char::is_ascii_digit) => {
                                return Err(invalid());
                            }
                            Some(_) => {}
                            None => return Err(invalid()),
                        }
                    }
                    size += 1;
                }
                Work::atom(size)?
            }
            '\\' => match rest.next().ok_or_else(invalid)? {
                ch @ ('k' | '1'..='9') => {
                    // Capturing a backreference could in turn make another
                    // capture wider, even with no repetition in the pattern.
                    if in_capture {
                        return Err(too_complex());
                    }
                    if ch == 'k' {
                        rest.next_if_eq(&'<').ok_or_else(invalid)?;
                        after_name(rest)?;
                    } else {
                        let mut index = ch.to_digit(10).unwrap();
                        while let Some(digit) = rest.next_if(char::is_ascii_digit) {
                            index = index * 10 + digit.to_digit(10).unwrap();
                            if index as usize > MAX_PATTERN_BYTES {
                                return Err(invalid());
                            }
                        }
                        syntax.max_numbered_ref = syntax.max_numbered_ref.max(index);
                    }
                    syntax.advanced = true;
                    Work::atom(MAX_CAPTURE_WORK + 1)?
                }
                'u' if rest.next_if_eq(&'{').is_some() => {
                    while rest.next_if(char::is_ascii_hexdigit).is_some() {}
                    rest.next_if_eq(&'}').ok_or_else(invalid)?;
                    Work::atom(1)?
                }
                'p' | 'P' => return Err(too_complex()),
                '0' if rest.peek().is_some_and(char::is_ascii_digit) => return Err(invalid()),
                'b' | 'B' => {
                    assertion = true;
                    Work::atom(1)?
                }
                _ => Work::atom(1)?,
            },
            '^' | '$' => {
                assertion = true;
                Work::atom(1)?
            }
            '*' | '+' | '?' | '{' | '}' => return Err(invalid()),
            _ => Work::atom(1)?,
        };
        let atom = match rest.next_if(|ch| matches!(ch, '*' | '+' | '?' | '{')) {
            None => atom,
            Some(_) if assertion => return Err(invalid()),
            Some('*' | '+') => return Err(too_complex()),
            Some(ch) => {
                let (min, max) = if ch == '?' {
                    (0, 1)
                } else {
                    let min = count(rest)?;
                    let max = if rest.next_if_eq(&',').is_some() {
                        count(rest)?
                    } else {
                        min
                    };
                    if max < min || rest.next_if_eq(&'}').is_none() {
                        return Err(invalid());
                    }
                    (min, max)
                };
                rest.next_if_eq(&'?');
                atom.repeat(min, max)?
            }
        };
        current = current.then(atom)?;
    }
    if let Some(branch) = alternatives {
        branch.or(current)
    } else {
        Ok(current)
    }
}

fn count(rest: &mut Input<'_>) -> Result<u32, SearchError> {
    let mut number = rest
        .next_if(char::is_ascii_digit)
        .ok_or_else(too_complex)?
        .to_digit(10)
        .unwrap();
    while let Some(digit) = rest.next_if(char::is_ascii_digit) {
        number = number * 10 + digit.to_digit(10).unwrap();
        if number > MAX_REPEAT {
            return Err(too_complex());
        }
    }
    Ok(number)
}

fn after_name(rest: &mut Input<'_>) -> Result<(), SearchError> {
    if rest.by_ref().any(|ch| ch == '>') {
        Ok(())
    } else {
        Err(invalid())
    }
}

fn invalid() -> SearchError {
    SearchError::InvalidRegExp("unsupported or invalid ECMAScript pattern".into())
}

fn too_complex() -> SearchError {
    SearchError::InvalidRegExp(
        "lookaround or backreference pattern has no small static work bound".into(),
    )
}
