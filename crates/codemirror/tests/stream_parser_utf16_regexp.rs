//! Frozen `StringStream.match` results from `@codemirror/streamparser` 6.0.0
//! on Node 26.10.0. The npm tarball matches `docs/codemirror-upstream.json`.
//! The Rust tests do not run JavaScript.

use std::num::NonZeroUsize;

use gpui_editor_codemirror::stream_parser::utf16::{
    RegExp, RegExpError, RegExpMatch, StringStream,
};
use serde_json::{Value, json};

const TAB: NonZeroUsize = NonZeroUsize::new(4).unwrap();
const LEAD: u16 = 0xd83e;
const TRAIL: u16 = 0xdd80;
const CRAB: &[u16] = &[LEAD, TRAIL];

#[test]
fn regexp_cases_match_pinned_streamparser() {
    let fixture = include_str!("fixtures/stream_parser_utf16_regexp.jsonl");
    for row in fixture.lines() {
        let case: Value = serde_json::from_str(row).unwrap();
        let source = case["source"].as_str().unwrap();
        let flags = case["flags"].as_str().unwrap();
        let regex = RegExp::new(source, flags);
        let Some(steps) = case["steps"].as_array() else {
            assert!(matches!(regex, Err(RegExpError::Pattern(_))), "{row}");
            continue;
        };
        let mut regex = regex.unwrap_or_else(|error| panic!("/{source}/{flags}: {error}"));
        let line: Vec<u16> = match case["text"].as_str() {
            Some(text) => text.encode_utf16().collect(),
            None => serde_json::from_value(case["text"].clone()).unwrap(),
        };
        let mut stream = StringStream::new(&line, TAB, 2);
        for step in steps {
            let result = match step[0].as_str().unwrap() {
                "setLast" => {
                    regex.last_index = step[1].as_i64().unwrap() as isize;
                    continue;
                }
                "setPos" => {
                    stream.pos = step[1].as_i64().unwrap() as isize;
                    continue;
                }
                "test" => json!(regex.test(&line)),
                "eat" => json!(stream.eat_regex(&mut regex)),
                "eatWhile" => json!(stream.eat_while_regex(&mut regex)),
                "match" => match stream.match_regex(&mut regex, step[1].as_bool() != Some(false)) {
                    Some(RegExpMatch::Single(matched)) => {
                        let captures: Vec<_> = matched.captures().collect();
                        for (number, &capture) in captures.iter().enumerate() {
                            assert_eq!(matched.get(number), capture, "{row}");
                        }
                        json!({
                            "captures": captures,
                            "index": matched.index(),
                            "input": matched.input(),
                            "names": matched.named_captures().collect::<Vec<_>>(),
                            "indices": matched.indices().map(|indices| indices.map(range_pair).collect::<Vec<_>>()),
                            "namedIndices": matched.named_indices().map(|indices| indices.map(|(name, range)| (name, range_pair(range))).collect::<Vec<_>>()),
                        })
                    }
                    Some(RegExpMatch::Global(matches)) => json!(matches),
                    None => Value::Null,
                },
                operation => panic!("unknown fixture operation: {operation}"),
            };
            assert_eq!(result, step[2], "{row}");
            assert_eq!(json!(stream.pos), step[3], "{row}");
            assert_eq!(json!(regex.last_index), step[4], "{row}");
        }
    }
}

fn range_pair(range: Option<std::ops::Range<usize>>) -> Option<[usize; 2]> {
    range.map(|range| [range.start, range.end])
}

fn assert_match(source: &[u16], flags: &str, line: &[u16], pos: usize, ranges: &[(usize, usize)]) {
    let context = || format!("/{}/{flags} on {line:x?}", String::from_utf16_lossy(source));
    let mut regex =
        RegExp::new_utf16(source, flags).unwrap_or_else(|error| panic!("{}: {error}", context()));
    let mut stream = StringStream::new(line, TAB, 2);
    stream.pos = pos as isize;
    stream.start_token();
    let matched = stream.match_regex(&mut regex, true);
    if let Some(&(start, end)) = ranges.first() {
        let Some(RegExpMatch::Single(matched)) = matched else {
            panic!("expected captures: {}", context());
        };
        assert_eq!(matched.index(), start, "{}", context());
        assert_eq!(matched.input(), &line[pos..], "{}", context());
        assert!(
            matched
                .indices()
                .unwrap()
                .eq(ranges.iter().map(|&(start, end)| Some(start..end))),
            "{}",
            context()
        );
        assert!(
            matched.captures().eq(ranges
                .iter()
                .map(|&(start, end)| Some(&line[pos + start..pos + end]))),
            "{}",
            context()
        );
        assert_eq!(stream.current(), &line[pos..pos + end], "{}", context());
    } else {
        assert!(matched.is_none(), "{}", context());
    }
    let end = ranges.first().map_or(0, |range| range.1) as isize;
    assert_eq!(stream.pos, pos as isize + end, "{}", context());
    assert_eq!(regex.last_index, if flags.contains('y') { end } else { 0 });
}

#[test]
fn unicode_surrogate_escapes_keep_the_next_atom_and_its_quantifier() {
    for flags in ["du", "dv"] {
        for (source, line, ranges) in [
            (r"^(\uD83E\u0041)$", &[LEAD, 65][..], &[(0, 2); 2][..]),
            (r"^(\uD83E\u{41}+)$", &[LEAD, 65, 65], &[(0, 3); 2]),
            (
                r"^(\uD83E\uD83E\uDD80+)$",
                &[LEAD, LEAD, TRAIL, LEAD, TRAIL],
                &[(0, 5); 2],
            ),
            (
                r"^(\uD83E\uDD80+)$",
                &[LEAD, TRAIL, LEAD, TRAIL],
                &[(0, 4); 2],
            ),
            (r"^(\uD83E\u{DD80})$", CRAB, &[]),
            (r"^(\u{D83E}\uDD80)$", CRAB, &[]),
            (r"^([\uD800-\uD83E\u{41}]+)$", &[LEAD, 65], &[(0, 2); 2]),
            (r"^([\uD83E\u0041]+)$", &[LEAD, 65], &[(0, 2); 2]),
            (r"^.(?<=\uD83E\u{41})(.)$", &[LEAD, 65, 33], &[]),
            (
                r"^..(?<=\uD83E\u{41})(.)$",
                &[LEAD, 65, 33],
                &[(0, 3), (2, 3)],
            ),
        ] {
            assert_match(
                &source.encode_utf16().collect::<Vec<_>>(),
                flags,
                line,
                0,
                ranges,
            );
        }
        let sticky = r"(\uD83E\u{41}+)".encode_utf16().collect::<Vec<_>>();
        assert_match(
            &sticky,
            &format!("{flags}y"),
            &[33, LEAD, 65, 65],
            1,
            &[(0, 3); 2],
        );
    }
}

#[test]
fn unicode_surrogate_pairing_depends_on_the_source_spelling() {
    for flags in ["du", "dv"] {
        for source in [
            "^(".encode_utf16()
                .chain([LEAD])
                .chain(r"\uDD80)$".encode_utf16())
                .collect::<Vec<_>>(),
            r"^(\uD83E"
                .encode_utf16()
                .chain([TRAIL])
                .chain(")$".encode_utf16())
                .collect(),
        ] {
            assert_match(&source, flags, CRAB, 0, &[]);
        }
        let pair = "^(🦀+)$".encode_utf16().collect::<Vec<_>>();
        assert_match(&pair, flags, &[LEAD, TRAIL, LEAD, TRAIL], 0, &[(0, 4); 2]);
        let literal = r"^\\uD83E\\u{2}$".encode_utf16().collect::<Vec<_>>();
        let line = r"\uD83E\uu".encode_utf16().collect::<Vec<_>>();
        assert_match(&literal, flags, &line, 0, &[(0, 9)]);
    }
}

#[test]
fn unicode_set_strings_keep_surrogates_backspaces_and_escaped_delimiters() {
    for flags in ["dv", "div"] {
        for (source, line, ranges) in [
            (
                r"^([\q{\uD83E\u{41}|\uD83E\u0042}])$",
                &[LEAD, 65][..],
                &[(0, 2); 2][..],
            ),
            (r"^([\q{\uD83E\uDD80a}])$", &[LEAD, TRAIL, 97], &[(0, 3); 2]),
            (r"^([\q{a\b|a\}|a\u{5b}}])$", &[97, 8], &[(0, 2); 2]),
            (r"^([\q{a\b|a\}|a\u{5b}}])$", &[97, 125], &[(0, 2); 2]),
            (r"^([\q{a\b|a\}|a\u{5b}}])$", &[97, 91], &[(0, 2); 2]),
            (r"^([\q{a\b|a\}|a\u{5b}}])$", &[97, 98], &[]),
            (r"^([\q{\\b}])$", &[92, 98], &[(0, 2); 2]),
            (r"^([[\q{\b}]&&[\x08]])$", &[8], &[(0, 1); 2]),
            (r"^([[\q{\b}]&&[\x08]])$", &[98], &[]),
        ] {
            assert_match(
                &source.encode_utf16().collect::<Vec<_>>(),
                flags,
                line,
                0,
                ranges,
            );
        }
    }
}

#[test]
fn negated_class_strings_classify_raw_surrogates_by_code_point() {
    let source = |body: &[u16]| {
        r"[^\q{"
            .encode_utf16()
            .chain(body.iter().copied())
            .chain("}]".encode_utf16())
            .collect::<Vec<_>>()
    };
    for flags in ["v", "iv"] {
        for body in [&[LEAD][..], &[TRAIL], CRAB, &[LEAD, 124, TRAIL]] {
            assert!(RegExp::new_utf16(&source(body), flags).is_ok(), "{body:x?}");
        }
        for body in [&[LEAD, 97][..], &[LEAD, TRAIL, 97], &[TRAIL, LEAD]] {
            assert!(
                matches!(
                    RegExp::new_utf16(&source(body), flags),
                    Err(RegExpError::Pattern(_))
                ),
                "{body:x?}"
            );
        }
    }
}

#[test]
fn word_boundary_assertions_reject_direct_quantifiers() {
    for flags in ["d", "di", "du", "diu", "dv", "div"] {
        for source in [r"\b+", r"\B*", r"\b?", r"\B{2}", r"\b{0,}", r"\B{1,2}?"] {
            assert!(
                matches!(RegExp::new(source, flags), Err(RegExpError::Pattern(_))),
                "/{source}/{flags}"
            );
        }
        for (source, line, ranges) in [
            (r"^(?:\b)+a$", &[97][..], &[(0, 1)][..]),
            (r"^[\b]+$", &[8, 8], &[(0, 2)]),
            (r"^\\b+$", &[92, 98, 98], &[(0, 3)]),
        ] {
            assert_match(
                &source.encode_utf16().collect::<Vec<_>>(),
                flags,
                line,
                0,
                ranges,
            );
        }
    }
    let source = r"^a\b{2x}$".encode_utf16().collect::<Vec<_>>();
    let line = "a{2x}".encode_utf16().collect::<Vec<_>>();
    assert_match(&source, "d", &line, 0, &[(0, 5)]);
}

#[test]
fn unicode_escape_rewrites_do_not_admit_invalid_patterns() {
    for flags in ["u", "iu", "v", "iv"] {
        for source in [
            r"\uD83E\u",
            r"\uD83E\uZZZZ",
            r"\uD83E\u{}",
            r"\uD83E\u+041",
            r"\u{+41}",
            r"(?<\u{+61}>a)",
            r"(?<\u+061>a)",
            r"\uD83E\u{110000}",
            r"[\uD83E\u{41}-\w]",
            r"(?<\uD83E\u0041>a)",
            r"\uD83E\u0041(",
            r"\uD83E\u0041\q",
            r"\uD83E\u0041{2,1}",
        ] {
            assert!(
                matches!(RegExp::new(source, flags), Err(RegExpError::Pattern(_))),
                "/{source}/{flags}"
            );
        }
    }
    for flags in ["v", "iv"] {
        for source in [r"[\q{\uD83E\u}]", r"[\q{\w\b}]", r"[\q{\b}-a]"] {
            assert!(
                matches!(RegExp::new(source, flags), Err(RegExpError::Pattern(_))),
                "/{source}/{flags}"
            );
        }
    }
}
