//! Pattern substitutions for specific `regress` differences.

use std::{borrow::Cow, cmp::Reverse};

mod case_fold;
mod composite_string_sets;

/// Adapt legacy escapes and simple character classes.
pub(super) fn legacy_pattern(
    pattern: &[u16],
    mut icase: bool,
) -> Result<Cow<'_, [u16]>, regress::Error> {
    let mut result = Rewrite::new(pattern);
    // Valid patterns this short have room for an extra group at any nesting
    // level. Length also bounds the work and output of the range rewrite.
    let ascii_classes = pattern.len() <= 256;
    let mut saved_flags = Vec::new();
    let mut class = false;
    let mut i = 0;
    while i < pattern.len() {
        match ascii(pattern, i) {
            Some(b'\\') => {
                let end = (i + 2).min(pattern.len());
                match ascii(pattern, i + 1) {
                    Some(b'b' | b'B') if !class => check_boundary_quantifier(pattern, end)?,
                    Some(word @ (b'w' | b'W')) if !class => {
                        result.replace(i, end, legacy_word_atom(word).encode_utf16());
                    }
                    Some(b'x') if hex(pattern, i + 2, 2).is_none() => {
                        // A failed hex escape does not consume trailing characters.
                        result.replace(i, end, r"\x78".encode_utf16());
                    }
                    Some(b'u') => match hex(pattern, i + 2, 4) {
                        // Leave the braces in \u{...} as a quantifier or literals.
                        None => result.replace(i, end, r"\x75".encode_utf16()),
                        // A raw lead surrogate cannot fuse with the next escape.
                        Some(unit @ 0xd800..=0xdbff) => {
                            result.replace(i, i + 6, [unit]);
                            i += 6;
                            continue;
                        }
                        Some(_) => {}
                    },
                    _ => {}
                }
                i = end;
                continue;
            }
            Some(b'[') if !class => {
                // Rewrite a class that consists of one word escape without
                // affecting the grammar of mixed classes.
                if let Some((escape, end)) = simple_word_class(pattern, i + 1) {
                    result.replace(i, end, legacy_word_atom(escape).encode_utf16());
                    i = end;
                    continue;
                }
                if ascii_classes
                    && icase
                    && let Some((end, atom)) = legacy_ascii_class(pattern, i + 1)
                {
                    result.replace(i, end, atom.encode_utf16());
                    i = end;
                    continue;
                }
                class = true;
            }
            Some(b']') => class = false,
            Some(b'(') if ascii_classes && !class => {
                saved_flags.push(icase);
                if let Some(value) = modified_icase(pattern, i + 1) {
                    icase = value;
                }
            }
            Some(b')') if ascii_classes && !class => {
                if let Some(value) = saved_flags.pop() {
                    icase = value;
                }
            }
            _ => {}
        }
        i += 1;
    }
    Ok(result.finish())
}

fn legacy_word_atom(escape: u8) -> &'static str {
    // Without a Unicode flag, WordCharacters contains only ASCII. Ignore
    // the effective case flag for this atom; both cases are already present.
    if escape == b'w' {
        "(?-i:[0-9A-Z_a-z])"
    } else {
        "(?-i:[^0-9A-Z_a-z])"
    }
}

fn simple_word_class(pattern: &[u16], start: usize) -> Option<(u8, usize)> {
    let invert = ascii(pattern, start) == Some(b'^');
    let start = start + usize::from(invert);
    if ascii(pattern, start) != Some(b'\\') || ascii(pattern, start + 2) != Some(b']') {
        return None;
    }
    let escape = match (ascii(pattern, start + 1)?, invert) {
        (b'w', false) | (b'W', true) => b'w',
        (b'W', false) | (b'w', true) => b'W',
        _ => return None,
    };
    Some((escape, start + 3))
}

fn legacy_ascii_class(pattern: &[u16], start: usize) -> Option<(usize, String)> {
    let negated = ascii(pattern, start) == Some(b'^');
    let mut at = start + usize::from(negated);
    let mut members = [false; 128];
    while ascii(pattern, at)? != b']' {
        if ascii(pattern, at) == Some(b'\\')
            && let Some(escape @ (b'd' | b'w')) = ascii(pattern, at + 1)
        {
            for (code, present) in members.iter_mut().enumerate() {
                let code = code as u8;
                *present |= code.is_ascii_digit()
                    || escape == b'w' && (code.is_ascii_alphabetic() || code == b'_');
            }
            at += 2;
            // A class escape used as a range endpoint has Annex B rules.
            if ascii_range_separator(pattern, at) {
                return None;
            }
            continue;
        }
        let (first, end) = ascii_class_literal(pattern, at)?;
        let (last, end) = if ascii_range_separator(pattern, end) {
            ascii_class_literal(pattern, end + 1)?
        } else {
            (first, end)
        };
        if first > last {
            return None;
        }
        for value in first..=last {
            members[usize::from(value.to_ascii_lowercase())] = true;
            members[usize::from(value.to_ascii_uppercase())] = true;
        }
        at = end;
    }
    if !members[usize::from(b'A')..=usize::from(b'Z')].contains(&true) {
        return None;
    }
    // Legacy canonicalization cannot fold non-ASCII units to ASCII. Apply
    // only ASCII case pairs, then disable backend folding for this class.
    let mut atom = if negated { "(?-i:[^" } else { "(?-i:[" }.to_owned();
    let mut code = 0;
    while code < members.len() {
        if !members[code] {
            code += 1;
            continue;
        }
        let first = code;
        while members.get(code + 1) == Some(&true) {
            code += 1;
        }
        atom.push_str(&format!(r"\x{first:02x}"));
        if code > first {
            atom.push_str(&format!(r"-\x{code:02x}"));
        }
        code += 1;
    }
    atom.push_str("])");
    Some((at + 1, atom))
}

fn ascii_range_separator(pattern: &[u16], at: usize) -> bool {
    ascii(pattern, at) == Some(b'-') && ascii(pattern, at + 1) != Some(b']')
}

fn ascii_class_literal(pattern: &[u16], at: usize) -> Option<(u8, usize)> {
    let byte = ascii(pattern, at).filter(u8::is_ascii)?;
    match byte {
        b']' => None,
        b'\\' => match ascii(pattern, at + 1)? {
            escape @ (b'x' | b'u') => {
                let digits = if escape == b'x' { 2 } else { 4 };
                let code = u8::try_from(hex(pattern, at + 2, digits)?).ok()?;
                code.is_ascii().then_some((code, at + 2 + digits))
            }
            escaped @ (b'\\' | b'[' | b']' | b'-' | b'^') => Some((escaped, at + 2)),
            _ => None,
        },
        _ => Some((byte, at + 1)),
    }
}

pub(super) fn hex(pattern: &[u16], start: usize, digits: usize) -> Option<u16> {
    pattern
        .get(start..start + digits)?
        .iter()
        .try_fold(0, |value, &unit| {
            let digit = char::from_u32(u32::from(unit))?.to_digit(16)?;
            Some(value * 16 + digit as u16)
        })
}

/// Make escape boundaries explicit before the backend checks the pattern.
pub(super) fn unicode_escapes(pattern: &[u16]) -> Result<Cow<'_, [u16]>, regress::Error> {
    let mut result = Rewrite::new(pattern);
    for i in escape_offsets(pattern) {
        if ascii(pattern, i + 1) != Some(b'u') {
            continue;
        }
        let digits = i + 2;
        if ascii(pattern, digits) == Some(b'+')
            || (ascii(pattern, digits) == Some(b'{') && ascii(pattern, digits + 1) == Some(b'+'))
        {
            // The backend's integer parser accepts signs, but HexDigits does not.
            return Err(regress::Error {
                text: "Invalid unicode escape".into(),
            });
        }
        let end = digits + 4;
        if let Some(lead @ 0xd800..=0xdbff) = hex(pattern, digits, 4)
            && ascii(pattern, end) == Some(b'\\')
            && ascii(pattern, end + 1) == Some(b'u')
            && !matches!(hex(pattern, end + 2, 4), Some(0xdc00..=0xdfff))
        {
            // The backend consumes the next `\u` even when it is not a trail
            // surrogate escape. A braced escape stops that lookahead without
            // changing the code point or the grammar of the following atom.
            result.replace(i, end, format!(r"\u{{{lead:04x}}}").encode_utf16());
        }
    }
    Ok(result.finish())
}

// ECMAScript WordCharacters in Unicode mode with IgnoreCase. Both sets are
// closed under simple case folding. The explicit complement can be inserted
// into a class without relying on how the backend folds a negated class.
const WORD: &str = r"0-9A-Z_a-z\u017f\u212a";
const NON_WORD: &str =
    r"\u0000-\x2f\x3a-\x40\x5b-\x5e\x60\x7b-\u017e\u0180-\u2129\u212b-\u{10ffff}";

#[derive(Default)]
enum SetOperation {
    #[default]
    Union,
    Intersection,
    Subtraction,
}

#[derive(Default)]
struct ClassSet {
    negated: bool,
    may_contain_strings: Option<bool>,
    operation: SetOperation,
}

impl ClassSet {
    fn add(&mut self, rhs: bool) {
        // ECMA-262 MayContainStrings is a property of the expression, not
        // its actual computed set. Two string operands can still be disjoint.
        self.may_contain_strings =
            Some(
                self.may_contain_strings
                    .map_or(rhs, |lhs| match self.operation {
                        SetOperation::Union => lhs || rhs,
                        SetOperation::Intersection => lhs && rhs,
                        SetOperation::Subtraction => lhs,
                    }),
            );
    }

    fn close(self) -> Result<bool, regress::Error> {
        let strings = self.may_contain_strings.unwrap_or(false);
        if self.negated && strings {
            return Err(regress::Error {
                text: "Negated Unicode set may contain strings".into(),
            });
        }
        Ok(strings)
    }
}

fn is_string_property(name: &[u16]) -> bool {
    [
        "Basic_Emoji",
        "Emoji_Keycap_Sequence",
        "RGI_Emoji_Modifier_Sequence",
        "RGI_Emoji_Flag_Sequence",
        "RGI_Emoji_Tag_Sequence",
        "RGI_Emoji_ZWJ_Sequence",
        "RGI_Emoji",
    ]
    .iter()
    .any(|property| name.iter().copied().eq(property.encode_utf16()))
}

/// Adapt Unicode ignore-case word escapes and some `v` class characters.
/// The caller checks syntax before class expansion, so an invalid range such
/// as `[a-\w]` cannot become valid.
pub(super) fn unicode_pattern(
    pattern: &[u16],
    sets: bool,
    mut icase: bool,
) -> Result<Cow<'_, [u16]>, regress::Error> {
    let mut result = Rewrite::new(pattern);
    let mut saved_flags = Vec::new();
    let mut classes = Vec::<ClassSet>::new();
    let composite_strings = sets && composite_string_sets::has_string_operand(pattern);
    let mut folded_budget = composite_string_sets::MAX_FOLDED_OUTPUT_UNITS;
    let mut i = 0;
    while i < pattern.len() {
        match ascii(pattern, i) {
            Some(b'\\') => {
                let end = (i + 2).min(pattern.len());
                let escape = ascii(pattern, i + 1);
                match escape {
                    Some(b'q')
                        if sets && !classes.is_empty() && ascii(pattern, end) == Some(b'{') =>
                    {
                        let start = i;
                        i = after_braces(pattern, end);
                        let body = &pattern[end..i];
                        let members = &body[1..body.len() - 1];
                        let strings = class_string_may_contain_strings(members);
                        let class = classes.last().expect("open class set");
                        let standalone = classes.len() == 1
                            && class.may_contain_strings.is_none()
                            && ascii(pattern, i) == Some(b']');
                        if standalone {
                            let negated = class.negated;
                            let replacement = if strings && !negated {
                                ordered_string_alternatives(members, icase)
                            } else if !strings && icase {
                                folded_singletons(members, negated)
                            } else {
                                None
                            };
                            if let Some(replacement) = replacement {
                                classes.pop();
                                i += 1;
                                result.replace(start - 1 - usize::from(negated), i, replacement);
                                continue;
                            }
                        }
                        classes.last_mut().expect("open class set").add(strings);
                        if !strings && !icase {
                            result.replace(start, i, singleton_set(members));
                            continue;
                        }
                        for offset in escape_offsets(body) {
                            if ascii(body, offset + 1) == Some(b'b') {
                                let start = end + offset;
                                result.replace(start, start + 2, r"\x08".encode_utf16());
                            }
                        }
                        continue;
                    }
                    Some(b'p' | b'P') if sets && ascii(pattern, end) == Some(b'{') => {
                        i = after_braces(pattern, end);
                        let strings =
                            escape == Some(b'p') && is_string_property(&pattern[end + 1..i - 1]);
                        if let Some(class) = classes.last_mut() {
                            class.add(strings);
                        }
                        continue;
                    }
                    Some(b'w' | b'W') if icase => {
                        let body = if escape == Some(b'w') { WORD } else { NON_WORD };
                        let brackets = sets || classes.is_empty();
                        let prefix = if brackets { "[" } else { "" };
                        let suffix = if brackets { "]" } else { "" };
                        result.replace(
                            i,
                            end,
                            [prefix, body, suffix]
                                .into_iter()
                                .flat_map(str::encode_utf16),
                        );
                    }
                    Some(b'b') if sets && !classes.is_empty() => {
                        // In a class, \b is a backspace, not an assertion.
                        result.replace(i, end, r"\x08".encode_utf16());
                    }
                    Some(b'b' | b'B') if classes.is_empty() => {
                        check_boundary_quantifier(pattern, end)?;
                        if !icase {
                            i = end;
                            continue;
                        }
                        // A boundary has a word character on exactly one side.
                        // Keep this an assertion, not a quantifiable empty atom.
                        let prefix = if escape == Some(b'b') { "(?=" } else { "(?!" };
                        let assertion =
                            format!("{prefix}(?<=[{WORD}])(?![{WORD}])|(?<![{WORD}])(?=[{WORD}]))");
                        result.replace(i, end, assertion.encode_utf16());
                    }
                    _ => {}
                }
                if let Some(class) = classes.last_mut() {
                    class.add(false);
                }
                i = end;
                continue;
            }
            Some(b'&') if sets && !classes.is_empty() => {
                if ascii(pattern, i + 1) == Some(b'&') {
                    if ascii(pattern, i + 2) == Some(b'&') {
                        return Err(regress::Error {
                            text: "Invalid Unicode set intersection".into(),
                        });
                    }
                    classes.last_mut().expect("open class set").operation =
                        SetOperation::Intersection;
                    i += 2;
                    continue;
                }
                classes.last_mut().expect("open class set").add(false);
                // The backend can drop the preceding union operand if it sees
                // a raw `&` instead of an intersection after that operand.
                result.replace(i, i + 1, r"\x26".encode_utf16());
            }
            Some(b'-') if sets && !classes.is_empty() && ascii(pattern, i + 1) == Some(b'-') => {
                classes.last_mut().expect("open class set").operation = SetOperation::Subtraction;
                i += 2;
                continue;
            }
            Some(b'[') if sets || classes.is_empty() => {
                if composite_strings
                    && classes.is_empty()
                    && (!icase
                        || saved_flags.len() <= composite_string_sets::MAX_FOLDED_SCOPE_DEPTH)
                    && let Some((end, replacement)) =
                        composite_string_sets::rewrite(pattern, i, icase.then_some(folded_budget))
                {
                    if icase {
                        folded_budget -= replacement.len();
                    }
                    result.replace(i, end, replacement);
                    i = end;
                    continue;
                }
                let negated = sets && ascii(pattern, i + 1) == Some(b'^');
                classes.push(ClassSet {
                    negated,
                    ..Default::default()
                });
                i += usize::from(negated);
            }
            Some(b']') if !classes.is_empty() => {
                let strings = classes.pop().expect("open class").close()?;
                if let Some(outer) = classes.last_mut() {
                    outer.add(strings);
                }
            }
            Some(b'(') if classes.is_empty() => {
                saved_flags.push(icase);
                if let Some(value) = modified_icase(pattern, i + 1) {
                    icase = value;
                }
            }
            Some(b')') if classes.is_empty() => {
                if let Some(value) = saved_flags.pop() {
                    icase = value;
                }
            }
            _ => {
                if let Some(class) = classes.last_mut() {
                    class.add(false);
                }
            }
        }
        i += 1;
    }
    Ok(result.finish())
}

// MayContainStrings (ECMA-262): an empty alternative or an alternative
// with more than one ClassSetCharacter makes a string disjunction ineligible
// for negation. The backend has already checked the escape syntax.
fn class_string_may_contain_strings(body: &[u16]) -> bool {
    let mut start = 0;
    loop {
        if start == body.len() || ascii(body, start) == Some(b'|') {
            return true;
        }
        let end = class_set_character_end(body, start);
        if end == body.len() {
            return false;
        }
        if ascii(body, end) != Some(b'|') {
            return true;
        }
        start = end + 1;
    }
}

// Keep case-sensitive singleton strings in one character-set operand so
// an intersection, subtraction, or complement applies to the whole set.
fn singleton_set(body: &[u16]) -> Vec<u16> {
    let mut result = vec![u16::from(b'[')];
    let mut start = 0;
    while start < body.len() {
        let end = class_set_character_end(body, start);
        write_class_character(&mut result, &body[start..end]);
        start = end + 1;
    }
    result.push(u16::from(b']'));
    result
}

// A complete positive class with just one string operand is an alternation.
// Put longer strings first and retain empty alternatives. Keep each literal
// in a singleton class so ClassSetCharacter escapes retain their meaning.
fn ordered_string_alternatives(body: &[u16], icase: bool) -> Option<Vec<u16>> {
    let mut alternatives = Vec::new();
    let mut literal = Vec::new();
    let mut length = 0;
    let mut start = 0;
    while start < body.len() {
        if ascii(body, start) == Some(b'|') {
            alternatives.push((length, std::mem::take(&mut literal)));
            length = 0;
            start += 1;
        } else {
            let end = class_set_character_end(body, start);
            let atom = body.get(start..end)?;
            let singleton = length == 0 && (end == body.len() || ascii(body, end) == Some(b'|'));
            if icase && singleton {
                literal.extend(folded_singletons(atom, false)?);
            } else {
                literal.push(u16::from(b'['));
                write_class_character(&mut literal, atom);
                literal.push(u16::from(b']'));
            }
            length += 1;
            start = end;
        }
    }
    alternatives.push((length, literal));
    alternatives.sort_by_key(|(length, _)| Reverse(*length));
    let mut result: Vec<_> = "(?:".encode_utf16().collect();
    for (index, (_, literal)) in alternatives.into_iter().enumerate() {
        if index > 0 {
            result.push(u16::from(b'|'));
        }
        result.extend(literal);
    }
    result.push(u16::from(b')'));
    Some(result)
}

// The pinned V8 folds singleton string operands themselves but then compares
// the input code point exactly. Disable `i` only for this generated class.
fn folded_singletons(body: &[u16], negated: bool) -> Option<Vec<u16>> {
    let mut result: Vec<_> = (if negated { "(?-i:[^" } else { "(?-i:[" })
        .encode_utf16()
        .collect();
    let mut start = 0;
    while start < body.len() {
        let end = class_set_character_end(body, start);
        let code = class_code_point(body.get(start..end)?)?;
        let folded = case_fold::fold(code);
        result.extend(format!(r"\u{{{folded:x}}}").encode_utf16());
        start = end + 1;
    }
    result.extend("])".encode_utf16());
    Some(result)
}

fn class_code_point(atom: &[u16]) -> Option<u32> {
    let pair =
        |lead, trail| 0x10000 + ((u32::from(lead) - 0xd800) << 10) + u32::from(trail) - 0xdc00;
    if super::is_surrogate_pair(atom) {
        return Some(pair(atom[0], atom[1]));
    }
    if ascii(atom, 0) != Some(b'\\') {
        return atom.first().copied().map(u32::from);
    }
    Some(match ascii(atom, 1)? {
        b'u' if ascii(atom, 2) == Some(b'{') => atom
            .get(3..atom.len().checked_sub(1)?)?
            .iter()
            .try_fold(0_u32, |value, &digit| {
                let digit = char::from_u32(u32::from(digit))?.to_digit(16)?;
                value.checked_mul(16)?.checked_add(digit)
            })?,
        b'u' if atom.len() == 12 => pair(hex(atom, 2, 4)?, hex(atom, 8, 4)?),
        b'u' => u32::from(hex(atom, 2, 4)?),
        b'x' => u32::from(hex(atom, 2, 2)?),
        b'c' => u32::from(ascii(atom, 2)?.to_ascii_uppercase() & 31),
        b'0' => 0,
        b'b' => 8,
        b't' => 9,
        b'n' => 10,
        b'v' => 11,
        b'f' => 12,
        b'r' => 13,
        punctuation if punctuation.is_ascii_punctuation() => u32::from(punctuation),
        _ => return None,
    })
}

fn write_class_character(result: &mut Vec<u16>, atom: &[u16]) {
    if let Some(scalar) = class_code_point(atom) {
        result.extend(format!(r"\u{{{scalar:x}}}").encode_utf16());
    } else {
        result.extend_from_slice(atom);
    }
}

fn class_set_character_end(body: &[u16], start: usize) -> usize {
    if ascii(body, start) != Some(b'\\') {
        return start + 1 + usize::from(super::is_surrogate_pair(&body[start..]));
    }
    let digits = start + 2;
    match ascii(body, start + 1) {
        Some(b'u') if ascii(body, digits) == Some(b'{') => after_braces(body, digits),
        Some(b'u') => {
            let end = digits + 4;
            // Only adjacent fixed-width escapes can form a surrogate pair.
            if matches!(hex(body, digits, 4), Some(0xd800..=0xdbff))
                && ascii(body, end) == Some(b'\\')
                && ascii(body, end + 1) == Some(b'u')
                && matches!(hex(body, end + 2, 4), Some(0xdc00..=0xdfff))
            {
                end + 6
            } else {
                end
            }
        }
        Some(b'x') => digits + 2,
        Some(b'c') => digits + 1,
        _ => digits,
    }
}

fn check_boundary_quantifier(pattern: &[u16], start: usize) -> Result<(), regress::Error> {
    // The backend can quantify a word boundary even though it is an assertion.
    // A malformed braced quantifier can still be literal text in legacy mode.
    let quantified = match ascii(pattern, start) {
        Some(b'*' | b'+' | b'?') => true,
        Some(b'{') => {
            let first = start + 1;
            let mut end = after_digits(pattern, first);
            if end == first {
                return Ok(());
            }
            if ascii(pattern, end) == Some(b',') {
                end = after_digits(pattern, end + 1);
            }
            ascii(pattern, end) == Some(b'}')
        }
        _ => false,
    };
    if quantified {
        return Err(regress::Error {
            text: "Quantifier not allowed here".into(),
        });
    }
    Ok(())
}

pub(super) fn after_digits(pattern: &[u16], mut i: usize) -> usize {
    while matches!(ascii(pattern, i), Some(b'0'..=b'9')) {
        i += 1;
    }
    i
}

// Find the end of a \q string. It can contain braced Unicode escapes.
pub(super) fn after_braces(pattern: &[u16], mut i: usize) -> usize {
    let mut depth = 0;
    while i < pattern.len() {
        match ascii(pattern, i) {
            Some(b'\\') if ascii(pattern, i + 1) != Some(b'u') => i += 1,
            Some(b'{') => depth += 1,
            Some(b'}') => {
                depth -= 1;
                if depth == 0 {
                    return i + 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
    pattern.len()
}

fn modified_icase(pattern: &[u16], start: usize) -> Option<bool> {
    if ascii(pattern, start) != Some(b'?') {
        return None;
    }
    let mut enabled = true;
    let mut icase = None;
    for i in start + 1..pattern.len() {
        match ascii(pattern, i) {
            Some(b'i') => icase = Some(enabled),
            Some(b'-') => enabled = false,
            Some(b'm' | b's') => {}
            Some(b':') => return icase,
            _ => break,
        }
    }
    None
}

fn escape_offsets(pattern: &[u16]) -> impl Iterator<Item = usize> + '_ {
    let mut start = 0;
    std::iter::from_fn(move || {
        let offset = start
            + pattern
                .get(start..)?
                .iter()
                .position(|&u| u == u16::from(b'\\'))?;
        start = offset + 2; // Do not process an escaped backslash again.
        Some(offset)
    })
}

pub(super) fn ascii(pattern: &[u16], i: usize) -> Option<u8> {
    u8::try_from(*pattern.get(i)?).ok()
}

pub(super) struct Rewrite<'a> {
    pattern: &'a [u16],
    written: usize,
    output: Vec<u16>,
}

impl<'a> Rewrite<'a> {
    pub(super) fn new(pattern: &'a [u16]) -> Self {
        Self {
            pattern,
            written: 0,
            output: Vec::new(),
        }
    }

    pub(super) fn replace(
        &mut self,
        start: usize,
        end: usize,
        replacement: impl IntoIterator<Item = u16>,
    ) {
        self.output
            .extend_from_slice(&self.pattern[self.written..start]);
        self.output.extend(replacement);
        self.written = end;
    }

    pub(super) fn finish(mut self) -> Cow<'a, [u16]> {
        if self.written == 0 {
            return Cow::Borrowed(self.pattern);
        }
        self.output.extend_from_slice(&self.pattern[self.written..]);
        Cow::Owned(self.output)
    }
}
