use gpui_editor_codemirror::{
    Text,
    language::brackets::{
        BracketConfig, BracketDirection::*, BracketMatch, bracket_near, bracket_near_in,
        match_bracket,
    },
};

fn pair(
    start: std::ops::Range<usize>,
    end: std::ops::Range<usize>,
    matched: bool,
) -> Option<BracketMatch> {
    Some(BracketMatch {
        start,
        end: Some(end),
        matched,
    })
}

#[test]
fn pinned_plain_brackets_scan_both_directions_and_handle_mismatches() {
    for (source, position, direction, expected) in [
        ("{[😀()]}", 0, Forward, pair(0..1, 9..10, true)),
        ("{[😀()]}", 10, Backward, pair(9..10, 0..1, true)),
        ("{[x])", 0, Forward, pair(0..1, 4..5, false)),
        (
            "{[x]",
            0,
            Forward,
            Some(BracketMatch {
                start: 0..1,
                end: None,
                matched: false,
            }),
        ),
        ("abc(", 1, Forward, None),
        ("({|})", 2, Backward, None),
    ] {
        assert_eq!(
            match_bracket(
                &Text::new(source),
                position,
                direction,
                BracketConfig::default(),
                |_, _| true
            ),
            expected
        );
    }
}

#[test]
fn pinned_distance_limit_is_checked_between_chunks_not_inside_a_line() {
    let config = BracketConfig {
        max_scan_distance: 1,
        ..BracketConfig::default()
    };
    let source = Text::new("{\nabc\n}");
    for (pos, dir) in [(0, Forward), (source.len(), Backward)] {
        assert_eq!(match_bracket(&source, pos, dir, config, |_, _| true), None);
    }
    assert_eq!(
        match_bracket(&Text::new("{abcdef}"), 0, Forward, config, |_, _| true),
        pair(0..1, 7..8, true)
    );
}

#[test]
fn custom_pairs_and_scope_predicate_map_to_utf8_ranges() {
    let config = BracketConfig {
        brackets: "«»‹›",
        ..BracketConfig::default()
    };
    let source = Text::new("«‹字›»");
    assert_eq!(
        match_bracket(&source, 0, Forward, config, |_, _| true),
        pair(0..2, 11..13, true)
    );
    assert_eq!(
        match_bracket(&source, 1, Forward, config, |_, _| true),
        None
    );
    assert_eq!(
        match_bracket(&source, source.len() + 1, Backward, config, |_, _| true),
        None
    );

    let source = Text::new("(\")\")");
    assert_eq!(
        match_bracket(
            &source,
            0,
            Forward,
            BracketConfig::default(),
            |start, pos| start == 0 && pos != 2
        ),
        pair(0..1, 4..5, true)
    );
}

#[test]
fn scoped_matching_uses_section_edges_and_rejects_invalid_carets() {
    let text = Text::new("外[(inside)字]");
    let config = BracketConfig::default();
    let start = "外".len();
    let content_end = text.len() - 1;
    let open = bracket_near_in(&text, start..content_end, start, config, |_, _| true).unwrap();
    assert_eq!(
        open,
        BracketMatch {
            start: start..start + 1,
            end: None,
            matched: false
        }
    );
    assert_eq!(
        bracket_near_in(&text, start + 1..content_end, start + 1, config, |_, _| {
            true
        }),
        pair(start + 1..start + 2, start + 8..start + 9, true)
    );
    assert_eq!(
        bracket_near_in(&text, start..content_end, 0, config, |_, _| true),
        None
    );
    assert_eq!(
        bracket_near_in(&text, 1..content_end, start, config, |_, _| true),
        None
    );
}

#[test]
fn caret_checks_the_nearest_bracket_and_can_disable_after_cursor() {
    let text = Text::new("x()[]");
    let config = BracketConfig::default();
    for (pos, expected) in [
        (1, pair(1..2, 2..3, true)),
        (2, pair(1..2, 2..3, true)),
        (3, pair(2..3, 1..2, true)),
        (4, pair(3..4, 4..5, true)),
        (5, pair(4..5, 3..4, true)),
    ] {
        assert_eq!(bracket_near(&text, pos, config, |_, _| true), expected);
    }
    assert_eq!(
        bracket_near(
            &text,
            1,
            BracketConfig {
                after_cursor: false,
                ..config
            },
            |_, _| true
        ),
        None
    );
}
