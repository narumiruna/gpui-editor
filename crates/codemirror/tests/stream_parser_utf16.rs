//! Frozen outputs from `@codemirror/streamparser` 6.0.0 `dist/index.js` on
//! Node 26.10.0. The npm archive matches the SHA-512 integrity digest in
//! `docs/codemirror-upstream.json`. The mode transcripts also use the pinned
//! `@codemirror/legacy-modes` 6.5.5 exports. These tests run only Rust.

use std::num::NonZeroUsize;

use gpui_editor_codemirror::stream_parser::utf16::{
    RegExp, RegExpError, RegExpMatch, StringStream,
};

const TAB: NonZeroUsize = NonZeroUsize::new(4).unwrap();
const CRAB: [u16; 2] = [0xd83e, 0xdd80];
const JS_SPACE: [u16; 25] = [
    0x9, 0xa, 0xb, 0xc, 0xd, 0x20, 0xa0, 0x1680, 0x2000, 0x2001, 0x2002, 0x2003, 0x2004, 0x2005,
    0x2006, 0x2007, 0x2008, 0x2009, 0x200a, 0x2028, 0x2029, 0x202f, 0x205f, 0x3000, 0xfeff,
];

fn utf16(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

#[test]
fn code_unit_cursor_preserves_surrogates_and_js_slice_bounds() {
    let line = utf16("🦀z");
    let mut stream = StringStream::new(&line, TAB, 2).with_indentation_override(0);
    assert_eq!(stream.indent_unit, 2);
    assert_eq!(stream.string, line);
    assert!(stream.sol());
    assert_eq!(stream.peek(), Some(&CRAB[..1]));
    assert_eq!(stream.eat(&CRAB), None);
    assert_eq!(stream.next(), Some(&CRAB[..1]));
    assert_eq!(stream.pos, 1);
    assert_eq!(stream.eat_if(|unit| unit == 0xdd80), Some(&CRAB[1..]));
    assert_eq!(stream.current(), CRAB);
    assert!(!stream.skip_to(&CRAB));
    stream.back_up(4);
    assert_eq!(stream.pos, -2);
    assert!(!stream.eol());
    assert!(!stream.sol());
    assert_eq!(stream.peek(), None);
    assert_eq!(stream.next(), Some(&[][..]));
    assert_eq!(stream.pos, -1);
    assert!(stream.match_str(&utf16("z"), false, false));
    let Some(RegExpMatch::Single(matched)) =
        stream.match_regex(&mut RegExp::new("^z", "").unwrap(), false)
    else {
        panic!("expected a prefix match in the relative suffix")
    };
    assert_eq!(matched.index(), 0);
    assert_eq!(matched.input(), utf16("z"));
    assert!(matched.indices().is_none());
    assert_eq!(stream.current(), CRAB);
    assert!(stream.skip_to(&CRAB[1..]));
    assert_eq!(stream.pos, 1);
    stream.start = -2;
    stream.skip_to_end();
    assert_eq!(stream.current(), &line[1..]);
    assert!(stream.eol());
    assert_eq!(stream.next(), None);
    assert_eq!(stream.eat_if(|_| panic!("predicate called at EOL")), None);
    assert_eq!(stream.eat(&[]), Some(&[][..]));
    assert_eq!(stream.pos, 4);
    assert!(stream.eat_while(&[]));
    assert_eq!(stream.pos, 5);
    assert!(stream.skip_to(&[]));
    assert_eq!(stream.pos, 3);
    assert_eq!(stream.indentation(), 0);

    let line = [0xd800, 0xd800, 0x007a];
    let mut stream = StringStream::new(&line, TAB, 2);
    assert!(stream.eat_while(&[0xd800]));
    assert_eq!(stream.current(), [0xd800, 0xd800]);
    assert!(stream.eat_while_if(|unit| unit == u16::from(b'z')));
    assert_eq!(stream.current(), line);
}

#[test]
fn js_whitespace_and_columns_count_units_and_keep_the_forward_only_cache() {
    let actual: Vec<_> = (0..=u16::MAX)
        .filter(|&unit| StringStream::new(&[unit], TAB, 2).eat_space())
        .collect();
    assert_eq!(actual, JS_SPACE);
    let mut line = utf16(" \t\u{a0}\u{feff}ᾨ\u{301}🦀");
    line.extend([0xd800, 9, 0x21]);
    let mut stream = StringStream::new(&line, NonZeroUsize::new(8).unwrap(), 3);
    assert_eq!(stream.indentation(), 10);
    assert!(stream.eat_space());
    assert_eq!(stream.pos, 4);
    assert_eq!(stream.column(), 0);
    for (pos, column) in [
        (4, 10),
        (7, 13),
        (8, 14),
        (9, 15),
        (10, 16),
        (11, 17),
        (1, 17),
        (12, 18),
    ] {
        stream.pos = pos;
        stream.start_token();
        assert_eq!(stream.column(), column);
    }
    let stream = stream.with_indentation_override(0);
    assert_eq!(stream.indentation(), 0);
    assert_eq!(stream.column(), 18);
}

#[test]
fn literal_match_lowers_only_the_utf16_prefix_even_when_casing_changes_length() {
    let samples = [
        (utf16("İ"), utf16("i\u{307}"), true, true, true, 2),
        (utf16("İx"), utf16("i"), true, true, false, 0),
        (utf16("ΟΣx"), utf16("ος"), true, true, true, 2),
        (utf16("ΟΣx"), utf16("οσ"), true, true, false, 0),
        (utf16("𐐀!"), utf16("𐐨"), true, true, true, 2),
        (CRAB.to_vec(), CRAB[..1].to_vec(), true, false, true, 1),
        (
            vec![0xd800, 0x3a3],
            vec![0xd800, 0x3c3],
            true,
            true,
            true,
            2,
        ),
        (vec![0xd800], vec![0xfffd], true, true, false, 0),
        (utf16("KéLAN"), utf16("KÉLAN"), false, true, true, 0),
        (Vec::new(), Vec::new(), true, false, true, 0),
    ];
    for (line, pattern, consume, caseless, found, pos) in samples {
        let mut stream = StringStream::new(&line, TAB, 2);
        assert_eq!(
            stream.match_str(&pattern, consume, caseless),
            found,
            "{line:x?}"
        );
        assert_eq!(stream.pos, pos);
        assert_eq!(stream.current(), &line[..line.len().min(pos as usize)]);
    }
}

#[test]
fn regexp_returns_captures_and_indices_in_the_original_units() {
    let line = utf16("!🦀42 α");
    let mut stream = StringStream::new(&line, TAB, 2);
    stream.pos = 1;
    stream.start_token();
    let mut pattern = RegExp::new(r"(?<pair>🦀)(\d+)(?<maybe>z)?", "du").unwrap();
    let Some(RegExpMatch::Single(matched)) = stream.match_regex(&mut pattern, false) else {
        panic!("expected non-global captures")
    };
    let expected = [Some(&line[1..5]), Some(&CRAB[..]), Some(&line[3..5]), None];
    assert!(matched.captures().eq(expected));
    assert_eq!(matched.index(), 0);
    assert_eq!(matched.input(), &line[1..]);
    assert!(
        matched
            .named_captures()
            .eq([("pair", Some(&CRAB[..])), ("maybe", None)])
    );
    assert!(
        matched
            .indices()
            .unwrap()
            .eq([Some(0..4), Some(0..2), Some(2..4), None])
    );
    assert!(
        matched
            .named_indices()
            .unwrap()
            .eq([("pair", Some(0..2)), ("maybe", None)])
    );
    assert_eq!(stream.pos, 1);
    assert!(stream.match_regex(&mut pattern, true).is_some());
    assert_eq!(stream.current(), &line[1..5]);
    assert_eq!(stream.pos, 5);

    stream.pos = 1;
    let mut lookahead = RegExp::new(r"(?=(?<ahead>🦀))", "du").unwrap();
    let Some(RegExpMatch::Single(matched)) = stream.match_regex(&mut lookahead, true) else {
        panic!("expected zero-length lookahead")
    };
    assert!(matched.indices().unwrap().eq([Some(0..0), Some(0..2)]));
    assert_eq!(stream.pos, 1);
}

#[test]
fn regexp_prefixes_use_ecmascript_syntax_and_unicode_modes() {
    let samples = [
        (r"(?<prefix>α)(?!z)\1", "du", utf16("αα!"), 0, Some(2)),
        (r"(?<=α)β", "u", utf16("αβ"), 1, None),
        (".", "d", CRAB.to_vec(), 0, Some(1)),
        (".", "du", CRAB.to_vec(), 0, Some(2)),
        (".", "du", CRAB.to_vec(), 1, Some(1)),
        ("🦀+", "d", utf16("🦀🦀"), 0, Some(2)),
        ("🦀+", "du", utf16("🦀🦀"), 0, Some(4)),
        (r"\uD83E\uDD80+", "du", utf16("🦀🦀"), 0, Some(4)),
        (r"\p{Surrogate}", "du", vec![0xdc00, 0x78], 0, Some(1)),
        (r"[\p{ASCII}&&\p{Letter}]+", "dv", utf16("abz1"), 0, Some(3)),
        (r"[\q{ab|a}]+", "dv", utf16("abaaZ"), 0, Some(4)),
        ("^(?i:a)B", "d", utf16("AB"), 0, Some(2)),
        ("^[a-z]+", "diu", utf16("Kſa"), 0, Some(3)),
        (r"\w+", "di", utf16("Kſa"), 0, None),
        (r"\bA\b", "diu", utf16("a!"), 0, Some(1)),
        (
            r"\s+",
            "d",
            utf16("\u{feff}\u{2028}\u{a0}\u{85}"),
            0,
            Some(3),
        ),
        (r"\s+", "d", utf16("\u{85}"), 0, None),
        (".+", "ds", utf16("\n\u{2028}x"), 0, Some(3)),
        (".+", "d", utf16("\n\u{2028}x"), 0, None),
        ("a$", "d", utf16("a\n"), 0, None),
        ("a$", "dm", utf16("a\u{2029}"), 0, Some(1)),
    ];
    for (source, flags, line, pos, size) in samples {
        let mut stream = StringStream::new(&line, TAB, 2);
        stream.pos = pos;
        let matched = stream.match_regex(&mut RegExp::new(source, flags).unwrap(), true);
        let text = matched.map(|matched| match matched {
            RegExpMatch::Single(matched) => {
                assert_eq!(matched.index(), 0);
                matched.get(0).unwrap()
            }
            RegExpMatch::Global(_) => panic!("unexpected global match"),
        });
        assert_eq!(
            text,
            size.map(|size| &line[pos as usize..pos as usize + size]),
            "/{source}/{flags}"
        );
        assert_eq!(stream.pos, pos + size.unwrap_or(0) as isize);
    }

    let mut surrogate = RegExp::new_utf16(&[0xdd80], "u").unwrap();
    assert!(surrogate.test(&[0xdd80]));
    assert!(!surrogate.test(&CRAB));
    let mut stream = StringStream::new(&CRAB, TAB, 2);
    stream.pos = 1;
    assert_eq!(stream.eat_regex(&mut surrogate), Some(&CRAB[1..]));
    for flags in ["uuv", "uv", "dd", "q"] {
        assert!(matches!(
            RegExp::new("", flags),
            Err(RegExpError::InvalidFlags)
        ));
    }
    assert!(matches!(RegExp::new("(", ""), Err(RegExpError::Pattern(_))));
}

#[test]
fn regexp_space_classes_match_the_pinned_bmp_sets() {
    for flags in ["", "i", "u", "iu", "v", "iv"] {
        for (source, positive) in [(r"^\s$", true), (r"^[\S]$", false)] {
            let mut regex = RegExp::new(source, flags).unwrap();
            let spaces: Vec<_> = (0..=u16::MAX)
                .filter(|&unit| regex.test(&[unit]) == positive)
                .collect();
            assert_eq!(spaces, JS_SPACE, "/{source}/{flags}");
        }
        for (source, positive) in [(r"^(?i:\s)$", true), (r"^(?-i:\S)$", false)] {
            let mut regex = RegExp::new(source, flags).unwrap();
            for unit in JS_SPACE.into_iter().chain([0x85, 0x180e, 0x200b, 0xd800]) {
                let is_space = JS_SPACE.contains(&unit);
                assert_eq!(
                    regex.test(&[unit]),
                    is_space == positive,
                    "/{source}/{flags}"
                );
            }
        }
    }
}

// The empty slice represents no match. A zero-length match still has a range.
fn assert_regex_ranges(source: &str, flags: &str, line: &[u16], ranges: &[(usize, usize)]) {
    let mut stream = StringStream::new(line, TAB, 2);
    let mut regex =
        RegExp::new(source, flags).unwrap_or_else(|error| panic!("/{source}/{flags}: {error}"));
    let matched = stream.match_regex(&mut regex, true);
    let context = || format!("/{source}/{flags} on {line:x?}");
    if let Some(&(_, end)) = ranges.first() {
        let Some(RegExpMatch::Single(matched)) = matched else {
            panic!("expected captures: {}", context());
        };
        assert_eq!(matched.index(), 0, "{}", context());
        assert_eq!(matched.input(), line, "{}", context());
        assert_eq!(stream.pos, end as isize, "{}", context());
        assert!(
            matched
                .indices()
                .unwrap()
                .eq(ranges.iter().map(|&(start, end)| Some(start..end))),
            "{}",
            context()
        );
        assert!(
            matched
                .captures()
                .eq(ranges.iter().map(|&(start, end)| Some(&line[start..end]))),
            "{}",
            context()
        );
    } else {
        assert!(matched.is_none(), "{}", context());
        assert_eq!(stream.pos, 0, "{}", context());
    }
    assert_eq!(regex.last_index, 0, "{}", context());
}

#[test]
fn regexp_word_classes_and_boundaries_follow_unicode_ignore_case() {
    for flags in ["d", "di", "du", "dv", "diu", "div"] {
        let folded_word = matches!(flags, "diu" | "div");
        let mut expected = (b'0'..=b'9')
            .chain(b'A'..=b'Z')
            .chain([b'_'])
            .chain(b'a'..=b'z')
            .map(u16::from)
            .collect::<Vec<_>>();
        if folded_word {
            expected.extend([0x17f, 0x212a]);
        }
        for (source, positive) in [(r"^\w$", true), (r"^[\W]$", false)] {
            let mut regex = RegExp::new(source, flags).unwrap();
            let words: Vec<_> = (0..=u16::MAX)
                .filter(|&unit| regex.test(&[unit]) == positive)
                .collect();
            assert_eq!(words, expected, "/{source}/{flags}");
        }
        for (source, text, ascii_ranges, folded_ranges) in [
            (r"\w+", "Kſa!", &[][..], &[(0, 3)][..]),
            (r"[\w]+", "Kſa!", &[][..], &[(0, 3)][..]),
            (r"\W+", "Kſé!", &[(0, 4)][..], &[][..]),
            (r"[\W]+", "Kſé!", &[(0, 4)][..], &[][..]),
            (r"^[^\w]+$", "Kſé", &[(0, 3)][..], &[][..]),
            (r"^[^\W]+$", "Kſa", &[][..], &[(0, 3)][..]),
            (r"\W+", "🦀-K", &[(0, 4)][..], &[(0, 3)][..]),
            (r"\bK\b", "K", &[][..], &[(0, 1)][..]),
            (r"\BK\B", "K", &[(0, 1)][..], &[][..]),
            (r"[\b]\w", "\x08ſ", &[][..], &[(0, 2)][..]),
        ] {
            let ranges = if folded_word {
                folded_ranges
            } else {
                ascii_ranges
            };
            assert_regex_ranges(source, flags, &utf16(text), ranges);
        }
    }
    for flags in ["diu", "div"] {
        for (source, text, ranges) in [
            (r"^(\b\w+\b)(\W)$", "Kſ🦀", &[(0, 4), (0, 2), (2, 4)][..]),
            (r"^(\W+)(?<=\W)\b\w+$", "🦀ſ", &[(0, 3), (0, 2)][..]),
            (r"^.(?<=\b\w)(?=\W)", "K!", &[(0, 1)][..]),
            (r"^[^\w]+$", "🦀é", &[(0, 3)][..]),
            (r"^\\w\W$", r"\w!", &[(0, 3)][..]),
            (r"^\b", "K", &[(0, 0)][..]),
            (r"^\B", "K", &[][..]),
        ] {
            assert_regex_ranges(source, flags, &utf16(text), ranges);
        }
        assert_regex_ranges(
            r"^(\W+)(\b\w+)$",
            flags,
            &[0xd800, 0x17f],
            &[(0, 2), (0, 1), (1, 2)],
        );
        for invalid in [r"\b+", r"[a-\w]"] {
            assert!(matches!(
                RegExp::new(invalid, flags),
                Err(RegExpError::Pattern(_))
            ));
        }
    }
}

#[test]
fn regexp_word_escapes_respect_group_scope_and_class_syntax() {
    for flags in ["du", "dv", "diu", "div"] {
        assert_regex_ranges(
            r"^(?i:(\b\w+\b))(?-i:(\B\W+\B))$",
            flags,
            &utf16("Kſ!"),
            &[(0, 3), (0, 2), (2, 3)],
        );
        assert_regex_ranges(
            r"^(?m-i:\B\W\B)!(?is:\b\w\b.)$",
            flags,
            &utf16("K!ſ\n"),
            &[(0, 4)],
        );
        assert_regex_ranges(
            r"^(?i:\w(?-i:(?i:\w))\w)!$",
            flags,
            &utf16("KſK!"),
            &[(0, 4)],
        );
    }
    for flags in ["d", "di", "du", "dv"] {
        let ranges = if flags.contains(['u', 'v']) {
            &[(0, 4)][..]
        } else {
            &[][..]
        };
        assert_regex_ranges(r"^(?i:\w(?-i:\W)\w)\W$", flags, &utf16("KKſK"), ranges);
    }
    assert_regex_ranges(r"^[(?i:]\w$", "du", &utf16("(K"), &[]);
    assert_regex_ranges(r"^[(?i:]\w$", "diu", &utf16("(K"), &[(0, 2)]);
    let sets = r"^[\q{a\}|a\u{5b}}][[\w]&&[\p{ASCII}ſK]]+";
    assert_regex_ranges(sets, "dv", &utf16("a}Kſa!"), &[]);
    assert_regex_ranges(sets, "div", &utf16("a}Kſa!"), &[(0, 5)]);
    assert_regex_ranges(r"^[[\W]&&[ſK\p{Letter}]]+", "div", &utf16("éKſ"), &[(0, 1)]);

    let line = utf16("Kſ!");
    let mut stream = StringStream::new(&line, TAB, 2);
    let mut pattern = RegExp::new(r"\b", "dgiu").unwrap();
    let Some(RegExpMatch::Global(matches)) = stream.match_regex(&mut pattern, true) else {
        panic!("expected two empty boundary matches");
    };
    assert_eq!(matches, [&line[..0]; 2]);
    assert_eq!((stream.pos, pattern.last_index), (0, 0));

    let mut pattern = RegExp::new(r"\b(\w+)\b", "diyu").unwrap();
    let line = utf16("🦀Kſ!");
    let mut stream = StringStream::new(&line, TAB, 2);
    pattern.last_index = 2;
    assert!(stream.match_regex(&mut pattern, true).is_none());
    assert_eq!((stream.pos, pattern.last_index), (0, 4));

    // The boundary sees the suffix start, not the preceding word character.
    let line = utf16("aKſ!");
    let mut stream = StringStream::new(&line, TAB, 2);
    stream.pos = 1;
    pattern.last_index = 0;
    let Some(RegExpMatch::Single(matched)) = stream.match_regex(&mut pattern, true) else {
        panic!("expected a boundary at the suffix start");
    };
    assert_eq!(matched.index(), 0);
    assert_eq!(matched.input(), &line[1..]);
    assert!(matched.captures().eq([Some(&line[1..3]); 2]));
    assert!(matched.indices().unwrap().eq([Some(0..2), Some(0..2)]));
    assert_eq!((stream.pos, pattern.last_index), (3, 2));
}

#[test]
fn regexp_legacy_escapes_keep_atom_boundaries_and_trailing_syntax() {
    for flags in ["d", "di", "du", "dv"] {
        let legacy = !flags.contains(['u', 'v']);
        let pair_end = if legacy { 2 } else { 4 };
        assert_regex_ranges(
            r"(\uD83E\uDD80+)",
            flags,
            &utf16("🦀🦀"),
            &[(0, pair_end), (0, pair_end)],
        );
        assert_regex_ranges(r"[\uD83E\uDD80]", flags, &CRAB, &[(0, pair_end / 2)]);
        assert_regex_ranges(
            r"\u{61}",
            flags,
            &utf16("aaaaaaaa"),
            if legacy { &[] } else { &[(0, 1)] },
        );
        assert_regex_ranges(
            r"^[\u{61}]+",
            flags,
            &utf16("u{61}a"),
            if legacy { &[(0, 5)] } else { &[] },
        );
        assert_regex_ranges(r"^\\xZ \\u{2}\\w", flags, &utf16(r"\xZ \uu\w"), &[(0, 9)]);
    }
    for flags in ["d", "di"] {
        for (source, text, ranges) in [
            (r"^(\u{2,4})!", "uuu!", &[(0, 4), (0, 3)][..]),
            (
                r"^(\xZ)(\x4)(\x41)",
                "xZx4A",
                &[(0, 5), (0, 2), (2, 4), (4, 5)][..],
            ),
            (r"^(?i:\xZ)(?-i:\xZ)$", "XzxZ", &[(0, 4)][..]),
            (r"[\xA-\x7a]+", "xyzA!", &[(0, 4)][..]),
            (r"[\x]", "x", &[(0, 1)][..]),
            (r"\x", "x", &[(0, 1)][..]),
            (r"\u4Z", "u4Z", &[(0, 3)][..]),
            (r"^\k<\u{2}>", "k<uu>", &[(0, 5)][..]),
            (r"[\uD83E\u0041]", "A", &[(0, 1)][..]),
        ] {
            assert_regex_ranges(source, flags, &utf16(text), ranges);
        }
        assert_regex_ranges(r"\u{61}", flags, &[u16::from(b'u'); 62], &[(0, 61)]);
        assert_regex_ranges(r"(\uD83E\u0041)", flags, &[0xd83e, 0x41], &[(0, 2), (0, 2)]);
    }
    assert_regex_ranges(r"\u+061", "d", &utf16("uu061"), &[(0, 5)]);
    for flags in ["du", "dv"] {
        for source in [r"\u{2,4}", r"\u4Z", r"\xZ", r"[\x]", r"\x"] {
            assert!(matches!(
                RegExp::new(source, flags),
                Err(RegExpError::Pattern(_))
            ));
        }
    }
}

#[test]
fn regexp_named_group_escapes_are_not_legacy_character_escapes() {
    for (source, text, capture_end) in [
        (r"^(?<\u{10400}>\xZ)\k<\uD801\uDC00>", "xZxZ", 2),
        (r"^\k<\u{10400}>(?<\uD801\uDC00>\x)$", "x", 1),
    ] {
        let mut pattern = RegExp::new(source, "d").unwrap();
        let line = utf16(text);
        let mut stream = StringStream::new(&line, TAB, 2);
        let Some(RegExpMatch::Single(matched)) = stream.match_regex(&mut pattern, true) else {
            panic!("expected a named capture and backreference in /{source}/d");
        };
        let capture = Some(&line[..capture_end]);
        assert!(matched.captures().eq([Some(&line[..]), capture]));
        assert!(
            matched
                .indices()
                .unwrap()
                .eq([Some(0..line.len()), Some(0..capture_end)])
        );
        assert!(matched.named_captures().eq([("𐐀", capture)]));
        assert!(
            matched
                .named_indices()
                .unwrap()
                .eq([("𐐀", Some(0..capture_end))])
        );
        assert_eq!(stream.pos, line.len() as isize);
    }
}

#[test]
fn regexp_uses_different_result_shapes_and_state_for_global_sticky_and_plain_matches() {
    for (flags, text, last, expected, pos) in [
        ("dg", "--xa-xb", 1, Some(vec!["xa", "xb"]), 2),
        ("dgy", "--xa-xb", 0, None, 0),
        ("dgy", "xaxb--", 0, Some(vec!["xa", "xb"]), 2),
    ] {
        let line = utf16(text);
        let mut stream = StringStream::new(&line, TAB, 2);
        let mut pattern = RegExp::new("x(.)", flags).unwrap();
        pattern.last_index = last;
        let matched = stream
            .match_regex(&mut pattern, true)
            .map(|matched| match matched {
                RegExpMatch::Global(matches) => matches,
                RegExpMatch::Single(_) => panic!("expected an index-free result"),
            });
        let expected = expected.map(|texts| texts.into_iter().map(utf16).collect::<Vec<_>>());
        assert_eq!(
            matched,
            expected
                .as_ref()
                .map(|texts| texts.iter().map(Vec::as_slice).collect())
        );
        assert_eq!(stream.pos, pos);
        assert_eq!(pattern.last_index, 0);
    }

    let line = utf16("xyx");
    let mut stream = StringStream::new(&line, TAB, 2);
    let mut sticky = RegExp::new("x", "dy").unwrap();
    sticky.last_index = 2;
    assert!(stream.match_regex(&mut sticky, true).is_none());
    assert_eq!((stream.pos, sticky.last_index), (0, 3));
    sticky.last_index = 1;
    assert!(stream.match_regex(&mut sticky, true).is_none());
    assert_eq!((stream.pos, sticky.last_index), (0, 0));
    sticky.last_index = -8;
    assert!(stream.match_regex(&mut sticky, false).is_some());
    assert_eq!((stream.pos, sticky.last_index), (0, 1));

    let line = utf16("🦀x");
    let mut stream = StringStream::new(&line, TAB, 2);
    let mut sticky = RegExp::new(".", "duy").unwrap();
    sticky.last_index = 1;
    assert!(stream.match_regex(&mut sticky, true).is_some());
    assert_eq!((stream.pos, sticky.last_index), (2, 2));

    for (flags, count) in [("dg", 3), ("dgu", 2), ("dgv", 2)] {
        let mut stream = StringStream::new(&CRAB, TAB, 2);
        let mut pattern = RegExp::new("(?:)", flags).unwrap();
        let Some(RegExpMatch::Global(matches)) = stream.match_regex(&mut pattern, true) else {
            panic!("expected empty global matches")
        };
        assert_eq!(matches.len(), count);
        assert!(matches.iter().all(|text| text.is_empty()));
        assert_eq!((stream.pos, pattern.last_index), (0, 0));
    }
}

#[test]
fn eat_regex_runs_stateful_test_on_one_unit_and_skips_test_at_eol() {
    let line = utf16("aa");
    for flags in ["g", "y", "gy", ""] {
        let mut stream = StringStream::new(&line, TAB, 2);
        let mut pattern = RegExp::new("a", flags).unwrap();
        assert!(stream.eat_while_regex(&mut pattern));
        assert_eq!(pattern.last_index, 0);
        if flags.is_empty() {
            assert_eq!(stream.pos, 2);
        } else {
            assert_eq!(stream.pos, 1);
            assert_eq!(stream.eat_regex(&mut pattern), Some(&line[1..]));
            assert_eq!(pattern.last_index, 1);
        }
        assert_eq!(stream.eat_regex(&mut pattern), None);
        assert_eq!(stream.pos, 2);
        assert_eq!(pattern.last_index, isize::from(!flags.is_empty()));
    }
}

fn check_line(
    line: &str,
    expected: &[(isize, isize, Option<&str>)],
    mut token: impl FnMut(&mut StringStream<'_>) -> Option<&'static str>,
) {
    let units = utf16(line);
    let mut stream = StringStream::new(&units, TAB, 2);
    for &(start, end, style) in expected {
        assert!(!stream.eol());
        stream.start_token();
        assert_eq!(stream.start, start);
        assert_eq!(token(&mut stream), style);
        assert_eq!(stream.pos, end);
        assert_eq!(stream.current(), &units[start as usize..end as usize]);
    }
    assert!(stream.eol());
}

#[test]
fn legacy_diff_transcript_uses_direct_cursor_assignment() {
    fn diff(stream: &mut StringStream<'_>, trailing_space: &mut RegExp) -> Option<&'static str> {
        let mut start = stream.string.len();
        while start > 0 && trailing_space.test(&stream.string[start - 1..start]) {
            start -= 1;
        }
        let first = stream
            .string
            .first()
            .copied()
            .and_then(|unit| u8::try_from(unit).ok());
        if !stream.sol() || start == 0 {
            stream.skip_to_end();
            return Some(match first {
                Some(b'+') => "error inserted",
                Some(b'-') => "error deleted",
                Some(b'@') => "error meta",
                _ => "error",
            });
        }
        stream.pos = start as isize;
        match first {
            Some(b'+') => Some("inserted"),
            Some(b'-') => Some("deleted"),
            Some(b'@') => Some("meta"),
            _ => None,
        }
    }
    let mut trailing_space = RegExp::new(r"[\t ]", "").unwrap();
    type Transcript<'a> = (&'a str, &'a [(isize, isize, Option<&'a str>)]);
    let transcript: &[Transcript<'_>] = &[
        (
            "+café🦀\t ",
            &[(0, 7, Some("inserted")), (7, 9, Some("error inserted"))],
        ),
        ("-\u{feff}🦀", &[(0, 4, Some("deleted"))]),
        ("@@@", &[(0, 3, Some("meta"))]),
        ("  ", &[(0, 2, Some("error"))]),
        (" plain", &[(0, 6, None)]),
        (" \t\u{a0}", &[(0, 3, None)]),
    ];
    for (line, expected) in transcript {
        check_line(line, expected, |stream| diff(stream, &mut trailing_space));
    }
}

#[test]
fn legacy_ascii_armor_transcript_uses_captures_whitespace_and_line_state() {
    enum State {
        Top,
        Headers,
        Header,
        Body,
        End,
    }
    let mut state = State::Top;
    let mut kind = None;
    let mut begin = RegExp::new(r"^-----BEGIN (.*)?-----\s*$", "").unwrap();
    let mut end = RegExp::new(r"^-----END (.*)?-----\s*$", "").unwrap();
    let mut header = RegExp::new(r"^\w+:", "").unwrap();
    let mut non_ws = RegExp::new(r"^\s*\S", "").unwrap();
    let mut base64 = RegExp::new(r"[A-Za-z0-9+/=]", "").unwrap();

    type Transcript<'a> = (&'a str, &'a [(isize, isize, Option<&'a str>)]);
    let transcript: &[Transcript<'_>] = &[
        ("\u{a0}\u{feff}", &[(0, 2, None)]),
        ("-----BEGIN TÉST🦀-----\t", &[(0, 23, Some("tag"))]),
        (
            "Comment: café🦀",
            &[(0, 8, Some("atom")), (8, 15, Some("string"))],
        ),
        ("", &[]),
        (
            "Y\u{85}Q==🦀",
            &[
                (0, 1, None),
                (1, 2, Some("error")),
                (2, 5, None),
                (5, 6, Some("error")),
                (6, 7, Some("error")),
            ],
        ),
        ("-----END WRONG-----", &[(0, 19, Some("error"))]),
        ("-----END TÉST🦀-----", &[(0, 20, Some("tag"))]),
        ("tail", &[(0, 4, Some("error"))]),
    ];
    for (line, expected) in transcript {
        if line.is_empty() && matches!(state, State::Headers) {
            state = State::Body;
        }
        check_line(line, expected, |stream| {
            match state {
                State::Top => {
                    if stream.sol()
                        && let Some(RegExpMatch::Single(matched)) =
                            stream.match_regex(&mut begin, true)
                    {
                        state = State::Headers;
                        kind = matched.get(1).map(<[u16]>::to_vec);
                        return Some("tag");
                    }
                }
                State::Headers => {
                    if stream.sol() && stream.match_regex(&mut header, true).is_some() {
                        state = State::Header;
                        return Some("atom");
                    }
                }
                State::Header => {
                    stream.skip_to_end();
                    state = State::Headers;
                    return Some("string");
                }
                State::Body => {
                    if stream.sol()
                        && let Some(RegExpMatch::Single(matched)) =
                            stream.match_regex(&mut end, true)
                    {
                        if matched.get(1) == kind.as_deref() {
                            state = State::End;
                            return Some("tag");
                        }
                        return Some("error");
                    }
                    if stream.eat_while_regex(&mut base64) {
                        return None;
                    }
                    stream.next();
                    return Some("error");
                }
                State::End => {}
            }
            let error = stream.match_regex(&mut non_ws, true).is_some();
            stream.skip_to_end();
            if error && matches!(state, State::Headers) {
                state = State::Body;
            }
            error.then_some("error")
        });
    }
}
