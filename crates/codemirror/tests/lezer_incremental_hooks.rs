use std::ops::Range;

use gpui_editor_codemirror::language::lezer::{
    ContextTracker, ExternalOptions, ExternalToken, ExternalTokenizer, GrammarSpec, ParseError,
    Parser, ReuseStats, SerializedTables, Specializer, StackView, SyntaxTree, TokenInput,
    Tokenizer, TreeFragment,
};

// Generated with @lezer/generator 1.8.1. Tree expectations were checked against
// @lezer/lr 1.4.13 and @lezer/common 1.5.7 with ranges converted to UTF-8 bytes.
// Tarballs were verified against docs/codemirror-upstream.json before use.
//
// @top Main { Item* (Part (Value | After) | Mark Right)? }
// Item { Plain | Marked | Astral | Change }
// Part { Aux }
// Aux { Mark }
// @external tokens probe from "hooks" { Change }
// @external tokens scan from "hooks" { Plain, Marked, Astral, Mark, Value, After, Right }
// @context track from "hooks"
const TABLES: SerializedTables<'_> = SerializedTables {
    version: 14,
    states: "!WQQOROOOOOR'#Cf'#CfOOOR'#Ci'#CiQQOROOOcOQO'#ChOOOQ'#Cg'#CgOnOQOOOOOR-E6g-E6gQOOOOO",
    state_data: "v~OPPOQPORPOSPOTSO~OWWOU[XV[X~OUWOVWO~O",
    goto: "q^PPPPPPPPPP_cgkTQORTUORTTORQRORVR",
    token_data: "RORO",
};
const CHANGE: u16 = 1;
const PLAIN: u16 = 2;
const MARKED: u16 = 3;
const ASTRAL: u16 = 4;
const MARK: u16 = 5;
const VALUE: u16 = 6;
const AFTER: u16 = 7;
const ITEM: u16 = 10;
const PART: u16 = 11;
const CARET: u16 = b'^' as u16;

#[derive(Clone, Copy, PartialEq)]
enum Probe {
    Peek,
    Advance,
    PeekEof,
    AdvanceEof,
    Clamp,
    UnusedEnd,
    Shift,
    Reduce,
    Branch,
    Work,
}

impl Probe {
    fn matches(self, input: &mut TokenInput<'_>) -> Result<bool, ParseError> {
        // The caret is at byte 64. Inspect the low surrogate of the scalar at
        // byte 200, or the same UTF-16 offset in an ASCII document.
        Ok(match self {
            Self::Advance => input.advance(137)? == Some(0xde00),
            Self::AdvanceEof => input.advance(137)?.is_none(),
            Self::PeekEof => input.peek(137)?.is_none(),
            Self::Clamp => {
                input.advance(usize::MAX)?;
                input.position() == 201
            }
            _ => input.peek(137)? == Some(0xde00),
        })
    }
}

type Scan = Result<Option<ExternalToken>, ParseError>;

fn scan(mode: Probe, input: &mut TokenInput<'_>, stack: &StackView<'_>) -> Scan {
    let context = stack.context::<u8>().copied().unwrap_or_default();
    let (term, len) = match input.peek(0)? {
        Some(CARET) if mode == Probe::Branch => (MARK, 1),
        Some(CARET) if mode == Probe::UnusedEnd => (PLAIN, 137),
        Some(112 | CARET) => (PLAIN, 1), // p, ^
        Some(119) if mode == Probe::Work => {
            for _ in 0..125 {
                input.peek(0)?;
            }
            (PLAIN, 1)
        }
        Some(122) => (if context != 0 { MARKED } else { PLAIN }, 1), // z
        Some(121) if input.peek(-1)? == Some(122) => (MARKED, 1),    // y after z
        Some(33) => (MARK, 1),                                       // !
        Some(120) => (if context == 2 { AFTER } else { VALUE }, 1),  // x
        Some(0xd83d) => (ASTRAL, 2),
        _ => return Ok(None),
    };
    // Deliberately do not read the following unit to make a token. The
    // runtime must still verify the absolute end from the UTF-16 index.
    Ok(Some(ExternalToken {
        term,
        end: input.position() + len,
    }))
}

fn parser(mode: Probe, opt_in: bool, specializers: &[Specializer]) -> Parser {
    let probe = move |input: &mut TokenInput<'_>, stack: &StackView<'_>| {
        if input.peek(0)? != Some(CARET) || !stack.can_shift(CHANGE)? {
            return Ok(None);
        }
        let end = input.position() + 1;
        let change = match mode {
            Probe::Shift | Probe::Reduce | Probe::Work => false,
            Probe::Branch | Probe::UnusedEnd => true,
            _ => mode.matches(input)?,
        };
        // Advance can scan much farther than the token it returns.
        Ok(change.then_some(ExternalToken { term: CHANGE, end }))
    };
    let options = ExternalOptions {
        contextual: true,
        extend: matches!(mode, Probe::Branch | Probe::UnusedEnd),
        ..ExternalOptions::default()
    };
    let probe = if opt_in {
        ExternalTokenizer::new_deterministic(options, probe)
    } else {
        ExternalTokenizer::new(options, probe)
    };
    let scan = ExternalTokenizer::new_deterministic(Default::default(), move |input, stack| {
        scan(mode, input, stack)
    });
    Parser::deserialize_with_hooks(
        GrammarSpec {
            tables: TABLES,
            node_names: "⚠ Change Plain Marked Astral Mark Value After Right Main Item Part Aux",
            max_term: 14,
            repeat_node_count: 1,
            tokenizers: &[Tokenizer::External(probe), Tokenizer::External(scan)],
            top_rule: [0, 9],
            token_prec: 0,
        },
        specializers,
    )
    .unwrap()
}

fn tracker(mode: Probe) -> ContextTracker<u8> {
    ContextTracker::new(0_u8)
        .with_shift(move |_, term, _, input| {
            if term == CHANGE
                || (mode == Probe::Shift
                    && term == PLAIN
                    && input.peek(0)? == Some(CARET)
                    && mode.matches(input)?)
            {
                return Ok(Some(1));
            }
            Ok(None)
        })
        .with_reduce(move |&context, term, _, input| {
            if term == PART && context == 1 {
                return Ok(Some(2));
            }
            if mode == Probe::Reduce && term == ITEM && input.peek(0)? == Some(CARET) {
                return Ok(mode.matches(input)?.then_some(1));
            }
            Ok(None)
        })
}

fn fragment(tree: &SyntaxTree, end: usize) -> TreeFragment<'_> {
    TreeFragment {
        tree,
        old_range: 0..end,
        new_range: 0..end,
    }
}

type Record<'a> = (u16, &'a str, Range<usize>, usize);

fn records(tree: &SyntaxTree) -> Vec<Record<'_>> {
    let mut result = Vec::new();
    let mut pending = vec![(tree.root(), 0)];
    while let Some((node, depth)) = pending.pop() {
        result.push((node.term(), node.name(), node.range(), depth));
        pending.extend(node.children().rev().map(|child| (child, depth + 1)));
    }
    result
}

fn edit(parser: &Parser, seed: &SyntaxTree, source: &str, cut: usize) -> SyntaxTree {
    let next = parser
        .parse_incremental(source, Some(fragment(seed, cut)))
        .unwrap();
    assert_eq!(records(&next), records(&parser.parse(source).unwrap()));
    next
}

fn assert_reuse(tree: &SyntaxTree, resumed_at: usize, nodes_reused: usize) {
    assert_eq!(
        tree.reuse_stats(),
        ReuseStats {
            resumed_at,
            nodes_reused,
        }
    );
}

fn prefix() -> String {
    format!("{}^{}", "p".repeat(64), "p".repeat(135))
}

fn assert_oracle(tree: &SyntaxTree, source: &str, caret: bool, marked: bool) {
    let mut expected = vec![(9, "Main", 0..source.len(), 0)];
    for (at, ch) in source.trim_end_matches("!x").char_indices() {
        let end = at + ch.len_utf8();
        let (term, name) = match ch {
            '^' if caret => (CHANGE, "Change"),
            'z' if marked => (MARKED, "Marked"),
            'y' => (MARKED, "Marked"),
            '😀' | '😁' => (ASTRAL, "Astral"),
            _ => (PLAIN, "Plain"),
        };
        expected.extend([(ITEM, "Item", at..end, 1), (term, name, at..end, 2)]);
    }
    if source.ends_with("!x") {
        let end = source.len();
        let mark = end - 2..end - 1;
        let (term, name) = if marked {
            (AFTER, "After")
        } else {
            (VALUE, "Value")
        };
        expected.extend([
            (PART, "Part", mark.clone(), 1),
            (12, "Aux", mark.clone(), 2),
            (MARK, "Mark", mark, 3),
            (term, name, end - 1..end, 1),
        ]);
    }
    assert_eq!(records(tree), expected);
}

#[test]
fn verifies_lookahead_and_restores_context_across_edits() {
    for mode in [Probe::Peek, Probe::Advance, Probe::Shift, Probe::Reduce] {
        let parser = parser(mode, true, &[]).with_deterministic_context_tracker(tracker(mode));
        let caret = matches!(mode, Probe::Peek | Probe::Advance);
        let prefix = prefix();
        let source = format!("{prefix}😀zy!x");
        let seed = parser.parse_incremental(&source, None).unwrap();
        assert_oracle(&seed, &source, caret, true);

        // A short-range edit leaves the lookahead and the context intact, and
        // both the cached Value and its replacement After run at the final x.
        let safe = format!("{prefix}😀zz!x");
        let reused = edit(&parser, &seed, &safe, 205);
        assert_reuse(&reused, 193, 385);
        assert_oracle(&reused, &safe, caret, true);
        assert_reuse(&edit(&parser, &seed, &safe, 204), 193, 385);

        // Branch back before the observed scalar, whose low surrogate changes.
        // Reduce runs after the checkpoint at the end of the caret's shift.
        let checkpoint = if mode == Probe::Reduce {
            (65, 129)
        } else {
            (1, 1)
        };
        let changed = format!("{prefix}😁zy!x");
        let next = edit(&parser, &reused, &changed, 200);
        assert_reuse(&next, checkpoint.0, checkpoint.1);
        assert_oracle(&next, &changed, false, false);
        assert_oracle(&seed, &source, caret, true);

        // The failed probe must also record the inspected unit. Changing the
        // same byte back after rebuilding the history cannot restore stale nodes.
        let back = edit(&parser, &next, &source, 200);
        assert_reuse(&back, checkpoint.0, checkpoint.1);
        assert_oracle(&back, &source, caret, true);
        assert_reuse(&edit(&parser, &seed, &source, 201), 0, 0);
    }
}

#[test]
fn reads_behind_the_resumed_cursor_after_a_multibyte_prefix() {
    let parser = parser(Probe::Peek, true, &[]);
    let prefix = format!("😀{}z", "p".repeat(191));
    let seed = parser
        .parse_incremental(&format!("{prefix}p!x"), None)
        .unwrap();
    let changed = format!("{prefix}y!x");
    let next = edit(&parser, &seed, &changed, prefix.len());
    assert_reuse(&next, 196, 385);
    assert_oracle(&next, &changed, false, false);
}

#[test]
fn validates_the_end_even_for_an_unselected_token() {
    let parser = parser(Probe::UnusedEnd, true, &[]);
    let prefix = prefix();
    let source = format!("{prefix}zz!x");
    // Change and Plain have the same packed action. Change wins, but Plain's
    // longer explicit end is still validated before duplicate actions are removed.
    let seed = parser.parse_incremental(&source, None).unwrap();
    assert_oracle(&seed, &source, true, false);
    assert_reuse(&edit(&parser, &seed, &source, 201), 193, 385);
    assert_reuse(&edit(&parser, &seed, &source, 200), 1, 1);

    let changed = format!("{prefix}😀zy!x");
    let error = ParseError::InvalidTokenEnd {
        offset: 64,
        end: 201,
    };
    assert_eq!(parser.parse(&changed).err(), Some(error));
    assert_eq!(
        parser
            .parse_incremental(&changed, Some(fragment(&seed, prefix.len())))
            .err(),
        Some(error),
    );
}

#[test]
fn observes_old_eof_from_peek_and_advance() {
    for mode in [Probe::PeekEof, Probe::AdvanceEof, Probe::Clamp] {
        let parser = parser(mode, true, &[]).with_deterministic_context_tracker(tracker(mode));
        let prefix = prefix();
        let source = format!("{prefix}z");
        let seed = parser.parse_incremental(&source, None).unwrap();
        assert_oracle(&seed, &source, true, true);
        assert_reuse(&edit(&parser, &seed, &source, source.len()), 193, 385);

        let appended = format!("{source}z!x");
        let next = edit(&parser, &seed, &appended, source.len());
        assert_reuse(&next, 1, 1);
        assert_oracle(&next, &appended, false, false);
    }
}

#[test]
fn does_not_reuse_across_ineligible_hooks_or_configuration_changes() {
    let mode = Probe::Peek;
    let source = format!("{}😀zy!x", prefix());
    for parser in [
        parser(mode, false, &[]).with_deterministic_context_tracker(tracker(mode)),
        parser(mode, true, &[]).with_context_tracker(tracker(mode)),
        parser(mode, true, &[Specializer::new(PLAIN, |_, _| Ok(None))])
            .with_deterministic_context_tracker(tracker(mode)),
    ] {
        let seed = parser.parse_incremental(&source, None).unwrap();
        assert_reuse(&edit(&parser, &seed, &source, source.len()), 0, 0);
    }

    let parser = parser(mode, true, &[]);
    let untracked = parser.parse_incremental(&source, None).unwrap();
    assert_reuse(&edit(&parser, &untracked, &source, source.len()), 193, 385);
    let parser = parser.with_deterministic_context_tracker(tracker(mode));
    let tracked = edit(&parser, &untracked, &source, source.len());
    assert_reuse(&tracked, 0, 0);
    assert_oracle(&tracked, &source, true, true);

    // Replacing an opted-in tracker must not restore an earlier context,
    // even when the Parser is moved rather than deserialized again.
    let parser = parser.with_deterministic_context_tracker(ContextTracker::new(false));
    assert_reuse(&edit(&parser, &tracked, &source, source.len()), 0, 0);
}

#[test]
fn keeps_dialect_configurations_separate() {
    let old =
        parser(Probe::Peek, true, &[]).with_deterministic_context_tracker(tracker(Probe::Peek));
    let new =
        parser(Probe::Work, true, &[]).with_deterministic_context_tracker(tracker(Probe::Work));
    let source = format!("{}😀zy!x", prefix());
    let seed = old.parse_incremental(&source, None).unwrap();
    assert_oracle(&seed, &source, true, true);
    let next = edit(&new, &seed, &source, source.len());
    assert_reuse(&next, 0, 0);
    assert_oracle(&next, &source, false, false);
}

#[test]
fn stops_at_ambiguity_and_does_not_reuse_branch_or_recovery_output() {
    let mode = Probe::Branch;
    let parser = parser(mode, true, &[]).with_deterministic_context_tracker(tracker(mode));
    let prefix = "p".repeat(200);
    let valid = format!("{prefix}p");
    let seed = parser.parse_incremental(&valid, None).unwrap();
    let ambiguity = format!("{prefix}^x");
    let error = parser.parse(&ambiguity).unwrap_err();
    assert!(matches!(error, ParseError::Ambiguous { offset: 200, .. }));
    assert_eq!(
        parser
            .parse_incremental(&ambiguity, Some(fragment(&seed, prefix.len())))
            .unwrap_err(),
        error,
    );
    let glr = parser.parse_glr(&ambiguity).unwrap();
    let recovered = parser.parse_recovering(&format!("{prefix}#")).unwrap();
    assert!(recovered.root().children().any(|node| node.is_error()));
    for non_lr in [&glr, &recovered] {
        assert_reuse(&edit(&parser, non_lr, &valid, prefix.len()), 0, 0);
    }
}

#[test]
fn recovered_seed_keeps_only_validated_records_after_earlier_arenas_drop() {
    let mode = Probe::Peek;
    let other = parser(mode, true, &[]).with_deterministic_context_tracker(tracker(mode));
    let parser = parser(mode, true, &[]).with_deterministic_context_tracker(tracker(mode));
    let prefix = prefix();
    let valid = format!("{prefix}😀zy!x");
    let seed = {
        let invalid = format!("{valid}#");
        let recovered = parser.parse_incremental_recovering(&invalid, &[]).unwrap();
        let fresh = parser.parse_recovering(&invalid).unwrap();
        assert_eq!(records(&recovered), records(&fresh));
        let again = parser
            .parse_incremental_recovering(&invalid, &[fragment(&recovered, invalid.len())])
            .unwrap();
        assert_eq!(records(&again), records(&fresh));
        assert_reuse(&again, 0, 0);
        assert_eq!(again.reused_suffix_actions(), 0);
        again
    };

    // The source and first recovered arena are gone. Later strict output
    // can still share the exact frozen records and restore the tracker.
    let repaired = edit(&parser, &seed, &valid, valid.len());
    assert_reuse(&repaired, 193, 385);
    assert_oracle(&repaired, &valid, true, true);

    let changed = format!("{prefix}😁zy!x");
    // Matching bytes through the edit do not cover the probe's read extent.
    let looked_ahead = edit(&parser, &seed, &changed, 200);
    assert_reuse(&looked_ahead, 1, 1);
    assert_oracle(&looked_ahead, &changed, false, false);
    // An inaccurate fragment cannot override validation of the actual bytes.
    assert_reuse(&edit(&parser, &seed, &changed, changed.len()), 0, 0);

    assert_reuse(&edit(&other, &seed, &valid, valid.len()), 0, 0);
}

#[test]
fn recovered_seed_checks_old_eof_and_never_records_after_strict_failure() {
    let mode = Probe::PeekEof;
    let parser = parser(mode, true, &[]).with_deterministic_context_tracker(tracker(mode));
    let invalid = format!("{}!", prefix());
    let seed = parser.parse_incremental_recovering(&invalid, &[]).unwrap();
    assert_eq!(
        records(&seed),
        records(&parser.parse_recovering(&invalid).unwrap())
    );
    let valid = format!("{invalid}x");
    let repaired = edit(&parser, &seed, &valid, invalid.len());
    assert_reuse(&repaired, 1, 1);
    assert_oracle(&repaired, &valid, false, false);

    // Recovery consumes the long tail, but strict parsing stops at byte 0.
    let invalid = format!("#{}", "p".repeat(300));
    let seed = parser.parse_incremental_recovering(&invalid, &[]).unwrap();
    assert!(seed.root().children().len() > 300);
    // Neither shifted suffix decisions nor a checkpoint after the error
    // can come from the recovery branch.
    let suffix = TreeFragment {
        tree: &seed,
        old_range: 1..invalid.len(),
        new_range: 0..invalid.len() - 1,
    };
    let without_error = parser
        .parse_incremental(&invalid[1..], Some(suffix))
        .unwrap();
    assert_reuse(&without_error, 0, 0);
    assert_eq!(without_error.reused_suffix_actions(), 0);
    let next = parser
        .parse_incremental_recovering(&invalid, &[fragment(&seed, invalid.len())])
        .unwrap();
    assert_eq!(records(&next), records(&seed));
    assert_reuse(&next, 0, 0);
    assert_eq!(next.reused_suffix_actions(), 0);
}

#[test]
fn charges_reused_scan_work_after_truncation() {
    let parser = parser(Probe::Work, true, &[]);
    let source = "w".repeat(256);
    let valid = format!("{source}{}", "p".repeat(256));
    let seed = parser.parse_incremental(&valid, None).unwrap();
    let recovered = parser
        .parse_incremental_recovering(&format!("{valid}#"), &[])
        .unwrap();
    let error = parser.parse(&source).unwrap_err();
    assert!(
        matches!(error, ParseError::LimitExceeded { offset } if offset > 193),
        "{error:?}"
    );
    for tree in [&seed, &recovered] {
        assert_eq!(
            parser
                .parse_incremental(&source, Some(fragment(tree, source.len())))
                .unwrap_err(),
            error,
        );
    }
}
