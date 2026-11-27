//! Strict fragment and fresh-parse snapshots from the pinned JavaScript runtime.
//! `upstreamReused` counts entries in its reused-tree buffer, not LR actions.
//! The native tests do not run JavaScript and do not expect the same reuse count.

use gpui_editor_codemirror::language::lezer::{
    ContextTracker, ExternalTokenizer, GrammarSpec, LocalTokenGroup, ParseError, Parser,
    ReuseStats, SerializedTables, Specialization, Specializer, SyntaxTree, Tokenizer, TreeFragment,
};
use serde_json::{Value, json};

fn fixture() -> Value {
    serde_json::from_str(include_str!("fixtures/lezer_shifted_suffix.json")).unwrap()
}

fn text(value: &Value) -> &str {
    value.as_str().unwrap()
}

fn number<T: TryFrom<u64>>(value: &Value) -> T {
    T::try_from(value.as_u64().unwrap()).ok().unwrap()
}

fn spec<'a, T>(table: &'a Value, tokenizers: &'a [T]) -> GrammarSpec<'a, T> {
    let top = &table["topRules"]["Main"];
    GrammarSpec {
        tables: SerializedTables {
            version: number(&table["version"]),
            states: text(&table["states"]),
            state_data: text(&table["stateData"]),
            goto: text(&table["goto"]),
            token_data: text(&table["tokenData"]),
        },
        node_names: text(&table["nodeNames"]),
        max_term: number(&table["maxTerm"]),
        repeat_node_count: number(&table["repeatNodeCount"]),
        tokenizers,
        top_rule: [number(&top[0]), number(&top[1])],
        token_prec: number(&table["tokenPrec"]),
    }
}

fn parser(table: &Value) -> Parser {
    let tokenizers: Vec<_> = table["tokenizers"]
        .as_array()
        .unwrap()
        .iter()
        .map(number::<u8>)
        .collect();
    Parser::deserialize(spec(table, &tokenizers)).unwrap()
}

fn records(tree: &SyntaxTree) -> Value {
    let mut records = Vec::new();
    let mut pending = vec![(tree.root(), 0)];
    while let Some((node, depth)) = pending.pop() {
        let range = node.range();
        records.push(json!([depth, node.name(), range.start, range.end]));
        pending.extend(node.children().rev().map(|child| (child, depth + 1)));
    }
    Value::Array(records)
}

fn shifted<'a>(
    tree: &'a SyntaxTree,
    before: &str,
    after: &str,
    [old_start, new_start]: [usize; 2],
) -> TreeFragment<'a> {
    TreeFragment {
        tree,
        old_range: old_start..before.len(),
        new_range: new_start..after.len(),
    }
}

fn prefix(tree: &SyntaxTree, end: usize) -> TreeFragment<'_> {
    TreeFragment {
        tree,
        old_range: 0..end,
        new_range: 0..end,
    }
}

fn oracle_shift(parser: &Parser, case: &Value) -> SyntaxTree {
    let before = text(&case["before"]);
    let after = text(&case["after"]);
    let starts = &case["suffixStarts"];
    let seed = parser.parse_incremental(before, None).unwrap();
    let next = parser
        .parse_incremental(
            after,
            Some(shifted(
                &seed,
                before,
                after,
                [number(&starts[0]), number(&starts[1])],
            )),
        )
        .unwrap();
    assert_eq!(records(&next), case["nodes"], "{after:?}");
    assert_eq!(records(&next), records(&parser.parse(after).unwrap()));
    assert_eq!(next.reuse_stats(), ReuseStats::default());
    next
}

#[test]
fn agrees_with_strict_upstream_fragments_after_shifts_and_state_changes() {
    let fixture = fixture();
    let parser = parser(&fixture["table"]);
    for name in ["insertion", "deletion", "stateChange"] {
        let next = oracle_shift(&parser, &fixture["cases"][name]);
        if name == "stateChange" {
            // Identical text comes from Word in Left and Pair in Right.
            assert_eq!(next.reused_suffix_actions(), 0);
        } else {
            assert!(next.reused_suffix_actions() > 5, "{name}: {next:?}");
        }
    }
}

#[test]
fn reuses_local_else_scans_with_immutable_specialization() {
    let fixture = fixture();
    let table = &fixture["localTable"];
    let local = &table["tokenizers"][1];
    let tokenizers = [
        Tokenizer::Internal(number(&table["tokenizers"][0])),
        Tokenizer::Local(
            LocalTokenGroup::new(
                text(&local["data"]),
                number(&local["precTable"]),
                Some(number(&local["elseToken"])),
            )
            .unwrap(),
        ),
    ];
    let entry = &table["specialized"][0];
    let result: u16 = number(&entry["result"]);
    assert_eq!(result & 1, 0);
    let specializer = Specializer::context_free(
        number(&entry["term"]),
        &[(text(&entry["text"]), Specialization::Replace(result >> 1))],
    );
    let parser = Parser::deserialize_with_hooks(spec(table, &tokenizers), &[specializer]).unwrap();
    let next = oracle_shift(&parser, &fixture["cases"]["localShift"]);
    assert!(next.reused_suffix_actions() > 10);
}

#[test]
fn shifted_decisions_can_seed_another_shift() {
    let fixture = fixture();
    let parser = parser(&fixture["table"]);
    let case = &fixture["cases"]["insertion"];
    let before = text(&case["before"]);
    let after = text(&case["after"]);
    let seed = parser.parse_incremental(before, None).unwrap();
    let next = parser
        .parse_incremental(after, Some(shifted(&seed, before, after, [2, 6])))
        .unwrap();
    let back = parser
        .parse_incremental(before, Some(shifted(&next, after, before, [6, 2])))
        .unwrap();
    assert!(back.reused_suffix_actions() > 5);
    assert_eq!(records(&back), records(&seed));
}

#[test]
fn combines_prefix_and_shifted_suffix_after_multiple_unicode_edits() {
    let fixture = fixture();
    let parser = parser(&fixture["table"]);
    let oracle: Value =
        serde_json::from_str(include_str!("fixtures/lezer_fragment_list.json")).unwrap();
    let [head, gap, tail] = ["head", "gap", "tail"].map(|key| text(&oracle[key]));
    let source = |edits: &Value| {
        let (left, right) = (text(&edits[0]), text(&edits[1]));
        format!("{head}{left}{gap}{right}{tail}")
    };
    let mut old_gap = head.len() + text(&oracle["initial"][0]).len();
    let mut before = source(&oracle["initial"]);
    let mut seed = parser.parse_incremental_fragments(&before, &[]).unwrap();

    for round in oracle["rounds"].as_array().unwrap() {
        let after = source(&round["edits"]);
        let starts = [before.len() - tail.len(), after.len() - tail.len()];
        let end = head.len();
        let prefix_only = parser
            .parse_incremental(&after, Some(prefix(&seed, end)))
            .unwrap();
        let suffix_only = parser
            .parse_incremental(&after, Some(shifted(&seed, &before, &after, starts)))
            .unwrap();
        assert!(prefix_only.reuse_stats().resumed_at >= 128);
        assert!(prefix_only.reuse_stats().nodes_reused > 50);
        assert_eq!(prefix_only.reused_suffix_actions(), 0);
        assert_eq!(suffix_only.reuse_stats(), ReuseStats::default());
        assert!(suffix_only.reused_suffix_actions() > 50);

        let new_gap = end + text(&round["edits"][0]).len();
        let mut fragments = [
            prefix(&seed, 60),
            shifted(&seed, &before, &after, starts.map(|start| start + 4)),
            TreeFragment {
                tree: &seed,
                old_range: old_gap..old_gap + gap.len(),
                new_range: new_gap..new_gap + gap.len(),
            },
            shifted(&seed, &before, &after, starts),
            prefix(&seed, end),
        ];
        let fresh = records(&parser.parse(&after).unwrap());
        assert_eq!(fresh, round["nodes"]);
        let verify = |fragments: &[TreeFragment<'_>]| {
            let next = parser
                .parse_incremental_fragments(&after, fragments)
                .unwrap();
            assert_eq!(records(&next), fresh);
            assert_eq!(next.reuse_stats(), prefix_only.reuse_stats());
            assert_eq!(
                next.reused_suffix_actions(),
                suffix_only.reused_suffix_actions()
            );
            next
        };
        verify(&fragments);
        fragments.reverse();
        seed = verify(&fragments);
        before = after;
        old_gap = new_gap;
    }
}

#[test]
fn invalid_list_entries_cannot_supply_or_hide_valid_reuse() {
    let fixture = fixture();
    let grammar = &fixture["table"];
    let first = parser(grammar);
    let other = parser(grammar);
    let head = format!("L 😀: {}", "ab: ".repeat(40));
    let tail = "ab: 😀: cd: ab: ef: gh: ij: kl:";
    let before = format!("{head}a: old: {tail}");
    let after = format!("{head}😀: new: {tail}");
    let starts = [before.len() - tail.len(), after.len() - tail.len()];
    let seed = first.parse_incremental(&before, None).unwrap();
    let foreign = other.parse_incremental(&before, None).unwrap();
    let plain = first.parse(&before).unwrap();
    let interior = TreeFragment {
        tree: &seed,
        old_range: starts[0]..before.len() - 1,
        new_range: starts[1]..after.len() - 1,
    };
    let mut fragments = vec![
        prefix(&seed, head.len() + 4), // Bytes differ.
        prefix(&seed, head.len() + 1), // Inside the astral scalar.
        prefix(&foreign, head.len()),
        prefix(&plain, head.len()),
        interior,
        shifted(&seed, &before, &after, starts.map(|start| start - 5)), // Bytes differ.
        shifted(&foreign, &before, &after, starts),
        shifted(&plain, &before, &after, starts),
        shifted(&seed, &before, &after, [before.len() + 1, after.len() + 1]),
    ];
    let fresh = records(&first.parse(&after).unwrap());
    let fallback = first
        .parse_incremental_fragments(&after, &fragments)
        .unwrap();
    assert_eq!(fallback.reuse_stats(), ReuseStats::default());
    assert_eq!(fallback.reused_suffix_actions(), 0);
    assert_eq!(records(&fallback), fresh);

    let valid = [
        prefix(&seed, head.len()),
        shifted(&seed, &before, &after, starts),
    ];
    let expected = first.parse_incremental_fragments(&after, &valid).unwrap();
    fragments.extend(valid);
    let mixed = first
        .parse_incremental_fragments(&after, &fragments)
        .unwrap();
    assert_eq!(mixed.reuse_stats(), expected.reuse_stats());
    assert!(mixed.reuse_stats().nodes_reused > 0);
    assert_eq!(
        mixed.reused_suffix_actions(),
        expected.reused_suffix_actions()
    );
    assert!(mixed.reused_suffix_actions() > 0);
    assert_eq!(records(&mixed), fresh);
}

#[test]
fn ignores_untrusted_suffixes_and_preserves_strict_errors() {
    let fixture = fixture();
    let grammar = &fixture["table"];
    let first = parser(grammar);
    let other = parser(grammar);
    let tracked = parser(grammar).with_deterministic_context_tracker(ContextTracker::new(()));
    let before = "L ab: 😀: cd:";
    let after = "L  ab: 😀: cd:";
    let seed = first.parse_incremental(before, None).unwrap();
    let plain = first.parse(before).unwrap();
    let recovered = first.parse_recovering(before).unwrap();
    let tracked_seed = tracked.parse_incremental(before, None).unwrap();
    let malformed: [([usize; 2], [usize; 2]); 5] = [
        ([2, before.len() - 1], [3, after.len() - 1]), // Not through EOF.
        ([7, before.len()], [8, after.len()]),         // Mid-scalar.
        ([2, before.len()], [4, after.len()]),         // Unequal bytes.
        ([2, before.len() + 1], [3, after.len()]),     // Out of bounds.
        ([before.len() + 1, before.len()], [3, after.len()]),
    ];
    let fresh = records(&first.parse(after).unwrap());
    for ([old_start, old_end], [new_start, new_end]) in malformed {
        let tree = first
            .parse_incremental(
                after,
                Some(TreeFragment {
                    tree: &seed,
                    old_range: old_start..old_end,
                    new_range: new_start..new_end,
                }),
            )
            .unwrap();
        assert_eq!(tree.reused_suffix_actions(), 0);
        assert_eq!(records(&tree), fresh);
    }
    for (parser, previous) in [
        (&other, &seed),
        (&tracked, &tracked_seed),
        (&first, &plain),
        (&first, &recovered),
    ] {
        let tree = parser
            .parse_incremental(after, Some(shifted(previous, before, after, [2, 3])))
            .unwrap();
        assert_eq!(tree.reused_suffix_actions(), 0);
        assert_eq!(records(&tree), fresh);
    }

    let changed = "L  ab: 😀: ce:";
    let tree = first
        .parse_incremental(changed, Some(shifted(&seed, before, changed, [2, 3])))
        .unwrap();
    assert_eq!(tree.reused_suffix_actions(), 0);
    assert_eq!(records(&tree), records(&first.parse(changed).unwrap()));

    // Upstream strict parse also rejects at byte 5. A matching suffix cannot
    // replay the Word action in the state that requires Pair followed by ':'.
    let before = "L abc:";
    let after = "R  abc:";
    let seed = first.parse_incremental(before, None).unwrap();
    let error = first.parse(after).unwrap_err();
    assert!(matches!(
        error,
        ParseError::UnexpectedToken { offset: 5, .. }
    ));
    assert_eq!(
        first
            .parse_incremental(after, Some(shifted(&seed, before, after, [2, 3])))
            .unwrap_err(),
        error,
    );
}

#[test]
fn does_not_reuse_suffix_scans_from_deterministic_external_tokenizers() {
    let fixture = fixture();
    let tokenizer = ExternalTokenizer::new_deterministic(Default::default(), |input, _| {
        let term = match input.peek(0)? {
            Some(120) if input.peek(-1)? == Some(98) => 2, // Short for x after b.
            Some(98 | 120) => 4,                           // Fill otherwise.
            _ => return Ok(None),
        };
        input.advance(1)?;
        Ok(Some(input.token(term)))
    });
    let parser = Parser::deserialize_with_hooks(
        spec(&fixture["lookTable"], &[Tokenizer::External(tokenizer)]),
        &[],
    )
    .unwrap();
    let seed = parser.parse_incremental("xx", None).unwrap();
    let next = parser
        .parse_incremental("bbx", Some(shifted(&seed, "xx", "bbx", [1, 2])))
        .unwrap();
    assert_eq!(next.reused_suffix_actions(), 0);
    assert_eq!(records(&next), records(&parser.parse("bbx").unwrap()));
    assert_eq!(next.root().children().next_back().unwrap().name(), "Short");
}

#[test]
fn reused_scans_keep_failed_lookahead_and_eof_dependencies() {
    let fixture = fixture();
    let parser = parser(&fixture["lookTable"]);
    let head = "x".repeat(160);
    let suffix = format!("a{}", "b".repeat(150));
    for stopper in ["", "x"] {
        let before = format!("{head}{suffix}{stopper}");
        let after = format!("{head}x{suffix}{stopper}");
        let start = head.len() + 1;
        let seed = parser.parse_incremental(&before, None).unwrap();
        let shifted = parser
            .parse_incremental_fragments(
                &after,
                &[
                    prefix(&seed, head.len()),
                    shifted(&seed, &before, &after, [head.len(), start]),
                ],
            )
            .unwrap();
        assert_eq!(records(&shifted), records(&parser.parse(&after).unwrap()));
        assert_eq!(shifted.reuse_stats().resumed_at, 129);
        assert_eq!(
            shifted.reused_suffix_actions(),
            suffix.len() + stopper.len()
        );
        assert_eq!(
            shifted.root().children().nth(start).unwrap().name(),
            "Short"
        );
        // A failed attempt to scan Long inspected the stopper, or saw EOF.
        // After the new byte arrives, all the b's form one Long token.
        let changed = format!("{head}x{suffix}c");
        let cut = changed.len() - 1;
        let next = parser
            .parse_incremental(&changed, Some(prefix(&shifted, cut)))
            .unwrap();
        assert_eq!(next.reuse_stats().resumed_at, 129);
        assert_eq!(records(&next), records(&parser.parse(&changed).unwrap()));
        let long = next.root().children().next_back().unwrap();
        assert_eq!(long.name(), "Long");
        assert_eq!(long.range(), start..changed.len());
    }
}

#[test]
fn charges_reused_lookahead_against_the_shorter_document_budget() {
    let fixture = fixture();
    let parser = parser(&fixture["lookTable"]);
    let head = "x".repeat(160);
    let body = format!("{}x", "a".repeat(480));
    let before = format!("{head}{}x{body}", " ".repeat(4096));
    let after = format!("{head}x{body}");
    let seed = parser.parse_incremental(&before, None).unwrap();
    let error = parser.parse(&after).unwrap_err();
    assert!(matches!(error, ParseError::LimitExceeded { .. }));
    assert_eq!(
        parser
            .parse_incremental_fragments(
                &after,
                &[
                    prefix(&seed, head.len()),
                    shifted(&seed, &before, &after, [head.len() + 4097, head.len() + 1]),
                ],
            )
            .unwrap_err(),
        error,
    );
}

#[test]
fn drops_shifted_decisions_when_strict_parsing_fails() {
    let fixture = fixture();
    let parser = parser(&fixture["lookTable"]);
    let invalid = format!("{}{}x", "x".repeat(161), "a".repeat(480));
    assert!(matches!(
        parser.parse(&invalid).unwrap_err(),
        ParseError::LimitExceeded { .. }
    ));
    let recovered = parser.parse_incremental_recovering(&invalid, &[]).unwrap();
    assert_eq!(
        records(&recovered),
        records(&parser.parse_recovering(&invalid).unwrap())
    );
    // Padding increases the strict scan budget. It leaves the state and the
    // early suffix bytes intact, but the failed run must not supply decisions.
    let padding = " ".repeat(4096);
    let valid = format!("{padding}{invalid}");
    let next = parser
        .parse_incremental(
            &valid,
            Some(shifted(&recovered, &invalid, &valid, [0, padding.len()])),
        )
        .unwrap();
    assert_eq!(records(&next), records(&parser.parse(&valid).unwrap()));
    assert_eq!(next.reused_suffix_actions(), 0);
}
