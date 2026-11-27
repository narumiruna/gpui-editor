//! Evaluate a bounded subset of positive `v` classes after backend syntax checking.

use std::{cmp::Reverse, collections::BTreeSet, fmt::Write, mem};

use super::{after_braces, ascii, class_code_point, class_set_character_end};

const STRING_OPERAND: [u16; 3] = [b'\\' as u16, b'q' as u16, b'{' as u16];
// A successful rewrite recompiles the whole pattern, not just this atom.
const MAX_PATTERN_UNITS: usize = 4096;
// Each member comes from the source. The span bounds total member storage and
// generated source size; the depth bounds how often a member can move up sets.
const MAX_CLASS_UNITS: usize = 1024;
const MAX_DEPTH: usize = 16;

type Members = BTreeSet<Vec<u32>>;

/// Skip the pass unless the source fits the recompile budget and contains `\q{`.
pub(super) fn has_string_operand(pattern: &[u16]) -> bool {
    pattern.len() <= MAX_PATTERN_UNITS && pattern.windows(3).any(|window| window == STRING_OPERAND)
}

/// Return the exclusive end and the replacement for a complete class atom.
/// The caller only probes outermost `v` classes with effective `i` disabled.
pub(super) fn rewrite(pattern: &[u16], start: usize) -> Option<(usize, Vec<u16>)> {
    let rest = pattern.get(start..)?;
    let source = &rest[..rest.len().min(MAX_CLASS_UNITS)];
    if ascii(source, 0) != Some(b'[') || !might_contain_string_operand(source) {
        return None;
    }
    let mut parser = Parser {
        source,
        pos: 0,
        composite: false,
        has_empty_or_long: false,
    };
    let members = parser.class(1)?;
    if !parser.composite || !parser.has_empty_or_long {
        return None;
    }

    // An empty set is a quantifiable atom that cannot match. A set containing
    // the empty string instead gets an empty alternative in the group.
    let end = start + parser.pos;
    if members.is_empty() {
        return Some((end, "[]".encode_utf16().collect()));
    }
    let mut members: Vec<_> = members.into_iter().collect();
    // Keep lexicographic order for equal-length strings. Separately spelled
    // surrogates can make two such strings match prefixes of different lengths.
    members.sort_by_key(|member| Reverse(member.len()));
    let mut replacement = String::from("(?:");
    for (index, member) in members.into_iter().enumerate() {
        if index > 0 {
            replacement.push('|');
        }
        let mut codes = member.into_iter().peekable();
        while let Some(mut code) = codes.next() {
            // Set operations compare the source code points. Node then matches
            // each surviving string as UTF-16, pairing adjacent halves here only.
            if (0xd800..=0xdbff).contains(&code)
                && let Some(trail) = codes.next_if(|trail| (0xdc00..=0xdfff).contains(trail))
            {
                code = 0x10000 + ((code - 0xd800) << 10) + (trail - 0xdc00);
            }
            if let Ok(byte) = u8::try_from(code)
                && byte.is_ascii_alphanumeric()
            {
                replacement.push(char::from(byte));
            } else {
                write!(replacement, r"\u{{{code:x}}}").expect("write to String");
            }
        }
    }
    replacement.push(')');
    Some((end, replacement.encode_utf16().collect()))
}

// Skip allocations for other class atoms in a pattern that uses `\q`.
// This is only a probe. The parser checks the complete atom before rewriting.
fn might_contain_string_operand(source: &[u16]) -> bool {
    let mut depth = 0;
    let mut pos = 0;
    while pos < source.len() {
        match ascii(source, pos) {
            Some(b'[') => depth += 1,
            Some(b']') => {
                depth -= 1;
                if depth == 0 {
                    return false;
                }
            }
            Some(b'\\') => {
                if source[pos..].starts_with(&STRING_OPERAND) {
                    // Let the existing path handle a standalone `\q` class.
                    return pos != 1 || ascii(source, after_braces(source, pos + 2)) != Some(b']');
                }
                pos += 1;
            }
            _ => {}
        }
        pos += 1;
    }
    false
}

#[derive(PartialEq, Eq)]
enum Operation {
    Union,
    Intersection,
    Difference,
}

impl Operation {
    fn apply(&self, mut left: Members, mut right: Members) -> Members {
        match self {
            Self::Union => {
                if left.len() < right.len() {
                    mem::swap(&mut left, &mut right);
                }
                left.extend(right);
            }
            Self::Intersection => {
                if left.len() < right.len() {
                    mem::swap(&mut left, &mut right);
                }
                right.retain(|member| left.contains(member));
                return right;
            }
            Self::Difference => {
                for member in right {
                    left.remove(&member);
                }
            }
        }
        left
    }
}

struct Parser<'a> {
    source: &'a [u16],
    pos: usize,
    composite: bool,
    has_empty_or_long: bool,
}

impl Parser<'_> {
    fn class(&mut self, depth: usize) -> Option<Members> {
        if depth > MAX_DEPTH || self.peek() != Some(b'[') {
            return None;
        }
        self.pos += 1;
        if self.peek() == Some(b'^') {
            return None;
        }
        if self.peek() == Some(b']') {
            self.pos += 1;
            return Some(Members::new());
        }
        let mut members = self.operand(depth)?;
        if self.peek() != Some(b']') {
            self.composite = true;
            let (operation, _) = self.operator();
            while self.peek() != Some(b']') {
                let (next, width) = self.operator();
                if operation != next {
                    return None;
                }
                self.pos += width;
                members = operation.apply(members, self.operand(depth)?);
            }
        }
        self.pos += 1;
        Some(members)
    }

    fn operator(&self) -> (Operation, usize) {
        match (self.peek(), ascii(self.source, self.pos + 1)) {
            (Some(b'&'), Some(b'&')) => (Operation::Intersection, 2),
            (Some(b'-'), Some(b'-')) => (Operation::Difference, 2),
            _ => (Operation::Union, 0),
        }
    }

    fn operand(&mut self, depth: usize) -> Option<Members> {
        if self.peek() == Some(b'[') {
            self.composite = true;
            return self.class(depth + 1);
        }
        if self.source[self.pos..].starts_with(&STRING_OPERAND) {
            self.pos += STRING_OPERAND.len();
            let mut members = Members::new();
            let mut member = Vec::new();
            loop {
                match self.peek() {
                    Some(end @ (b'|' | b'}')) => {
                        self.has_empty_or_long |= member.len() != 1;
                        members.insert(mem::take(&mut member));
                        self.pos += 1;
                        if end == b'}' {
                            return Some(members);
                        }
                    }
                    _ => member.push(self.character()?),
                }
            }
        }
        Some(Members::from([vec![self.character()?]]))
    }

    fn character(&mut self) -> Option<u32> {
        // The backend accepts some invalid class-set characters. Do not turn
        // them into valid literal escapes in the rewritten atom.
        match self.peek() {
            Some(b'\\') => match ascii(self.source, self.pos + 1)? {
                b'u' | b'x' | b'b' | b'f' | b'n' | b'r' | b't' | b'v' => {}
                b'0' if !ascii(self.source, self.pos + 2).is_some_and(|b| b.is_ascii_digit()) => {}
                b'c' if ascii(self.source, self.pos + 2)
                    .is_some_and(|b| b.is_ascii_alphabetic()) => {}
                punctuation if b"^$\\.*+?()[]{}|/&-!#%,:;<=>@`~".contains(&punctuation) => {}
                _ => return None,
            },
            Some(b'(' | b')' | b'[' | b']' | b'{' | b'}' | b'/' | b'-' | b'|') => return None,
            Some(b)
                if b"!#$%&*+,.:;<=>?@^`~".contains(&b)
                    && ascii(self.source, self.pos + 1) == Some(b) =>
            {
                return None;
            }
            _ => {}
        }
        self.source.get(self.pos)?;
        let end = class_set_character_end(self.source, self.pos);
        let atom = self.source.get(self.pos..end)?;
        if atom.starts_with(&[b'\\' as u16, b'u' as u16, b'{' as u16])
            && (atom.len() < 5 || atom.last() != Some(&u16::from(b'}')))
        {
            return None;
        }
        let code = class_code_point(atom).filter(|&code| code <= 0x10ffff)?;
        self.pos = end;
        Some(code)
    }

    fn peek(&self) -> Option<u8> {
        ascii(self.source, self.pos)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rewrite_class(source: &str) -> Option<(usize, Vec<u16>)> {
        rewrite(&source.encode_utf16().collect::<Vec<_>>(), 0)
    }

    #[test]
    fn composite_string_sets_leave_unsupported_or_invalid_atoms_unchanged() {
        for source in [
            r"[\q{a|}]",
            r"[[\q{a}][\q{b}]]",
            r"[a][[\q{a|}]]",
            r"[\\q\{a\}][[\q{a|}]]",
            r"[^[[\q{a|}]]]",
            r"[[^x][\q{a|}]]",
            r"[\q{a|ab}&&[a-z]]",
            r"[\q{a|ab}&&\d]",
            r"[\q{a|ab}&&\p{RGI_Emoji}]",
            r"[[]&&[\q{a|}]&&\p{ASCII}]",
            r"[[\q{a|}]&&[\q{b|}]--[\q{c|}]]",
            r"[[\q{a|}]&&[\q{b|}]x]",
            r"[[\q{a|}]&&&[\q{b|}]]",
            r"[[\q{a|}]a--[\q{b|}]]",
            r"[[\q{a|}]--]",
            r"[[\q{a|}]--[\q{b|}]",
            r"[[\q{a[|}]]",
            r"[[\q{a]|}]]",
            r"[[\q{a/|}]]",
            r"[[\q{aa||!!}]]",
            r"[[\q{\01|}]]",
            r"[[\q{\_|}]]",
            r"[[\q{\c1|}]]",
            r"[[\q{\u{}|}]]",
            r"[[\q{\u{110000}|}]]",
            r"[[\q{\x+1|}]]",
        ] {
            assert!(rewrite_class(source).is_none(), "{source}");
        }
    }

    #[test]
    fn composite_string_sets_bound_recompile_size_probe_span_and_depth() {
        let mut pattern = vec![u16::from(b'a'); MAX_PATTERN_UNITS - STRING_OPERAND.len()];
        pattern.extend(STRING_OPERAND);
        assert!(has_string_operand(&pattern));
        pattern.push(u16::from(b'a'));
        assert!(!has_string_operand(&pattern));

        let nesting = |depth| format!(r"{}\q{{ab|}}{}", "[".repeat(depth), "]".repeat(depth));
        assert!(rewrite_class(&nesting(MAX_DEPTH)).is_some());
        assert!(rewrite_class(&nesting(MAX_DEPTH + 1)).is_none());
        assert!(rewrite_class(&nesting(10_000)).is_none());

        let prefix = r"[[\q{";
        let suffix = "|}]]";
        let body = "a".repeat(MAX_CLASS_UNITS - prefix.len() - suffix.len());
        let source = format!("{prefix}{body}{suffix}");
        let (end, rewritten) = rewrite_class(&format!("{source}+b")).unwrap();
        assert_eq!(end, MAX_CLASS_UNITS);
        assert!(rewritten.len() <= MAX_CLASS_UNITS * 10);
        assert!(rewrite_class(&format!("{prefix}{body}a{suffix}")).is_none());
    }
}
