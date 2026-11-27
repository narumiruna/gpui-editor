use super::{
    EDITOR_FRONTIER, EDITOR_MAX_BRANCHES, ParseError, Parser, SyntaxTree, TokenCache, TreeBuilder,
    Work,
};
use crate::language::lezer::tree::Roots;
use crate::language::lezer::{ExternalOptions, ExternalTokenizer, Tables, Tokenizer};

const SOURCE: &str = "xxxxxy";

// The first token shifts to each of `branches` distinct states. Only one
// state can scan the second token. All shifts leave the branch score at zero.
fn fork_parser(branches: u16, second: u16, reads: usize) -> Parser {
    let mut states = vec![0, 2, 0, 1, 0, 0];
    let mut actions = vec![u16::MAX, 0];
    for target in 1..=branches {
        actions.extend([2, target, 0]);
    }
    actions.extend([u16::MAX, 0]);
    let rest = actions.len() as u32;
    actions.extend([3, branches + 1, 0, u16::MAX, 0]);
    for _ in 0..branches {
        states.extend([0, rest, 0, 1, 0, 0]);
    }
    states.extend([2, 0, 0, 0, 0, 0]);
    Parser {
        tables: Tables {
            states,
            state_data: actions,
            goto: vec![0],
            token_data: vec![],
        },
        names: ["⚠", "Main", "First", "Second"]
            .into_iter()
            .map(Box::from)
            .collect(),
        top_rule: [0, 1],
        eof: 4,
        max_term: 4,
        tokenizers: Box::new([Tokenizer::External(ExternalTokenizer::new(
            ExternalOptions {
                contextual: true,
                ..ExternalOptions::default()
            },
            move |input, stack| {
                if stack.state() == 0 {
                    input.advance(5)?;
                    return Ok(Some(input.token(2)));
                }
                for _ in 0..reads {
                    input.peek(0)?;
                }
                if stack.state() == second && input.peek(0)? == Some(u16::from(b'y')) {
                    input.advance(1)?;
                    return Ok(Some(input.token(3)));
                }
                Ok(None)
            },
        ))]),
        precedence: Box::new([]),
        specializers: Box::new([]),
        dynamic_precedence: Box::new([]),
        context_tracker: None,
    }
}

#[test]
fn strict_reduction_goto_cache_checks_keys_and_meters_hits_and_misses() {
    let mut parser = fork_parser(32, 1, 0);
    // Term 1 has two strict goto groups: source 0 takes target 1, and
    // source 32 takes target 2. Both keys map to the same cache slot.
    parser.tables.goto = vec![2, 1, 3, 2, 1, 0, 3, 2, 32];
    let mut tokens = TokenCache::new(SOURCE, &parser, false);
    tokens.reduction_gotos = Some(Box::default());
    let mut lookup =
        |state, units| parser.reduction_goto(state, 1, 5, &Work::new(units), &mut tokens);
    let exhausted = Err(ParseError::LimitExceeded { offset: 5 });
    assert_eq!(lookup(0, 3), exhausted);
    assert_eq!(lookup(0, 4), Ok(Some(1)));
    assert_eq!(lookup(0, 0), exhausted);
    assert_eq!(lookup(0, 1), Ok(Some(1)));
    assert_eq!(lookup(32, 6), exhausted);
    assert_eq!(lookup(0, 1), Ok(Some(1)));
    assert_eq!(lookup(32, 7), Ok(Some(2)));
    assert_eq!(lookup(32, 1), Ok(Some(2)));
    assert_eq!(lookup(0, 1), exhausted);
    assert_eq!(lookup(0, 4), Ok(Some(1)));
    assert_eq!(parser.tables.goto(1, 1, true), Some(2));
    assert_eq!(lookup(1, 7), Ok(None));
    assert_eq!(lookup(1, 1), exhausted);
}

#[test]
fn deferred_reductions_charge_for_retained_children_and_discard_dead_lists() {
    let parser = fork_parser(1, 1, 0);
    for (budget, succeeds) in [(12, false), (13, true)] {
        let mut tree = TreeBuilder::for_glr(true, parser.names.len());
        let base = Roots::default();
        let mut roots = base;
        for at in 0..3 {
            tree.leaf(&mut roots, 2, at..at + 1);
        }
        let mut discarded = roots;
        let work = Work::new(2);
        tree.reduce(&mut discarded, base, 3, 0..3, 3, &work)
            .unwrap();
        tree.reduce(&mut roots, base, 3, 0..3, 3, &work).unwrap();
        assert_eq!(work.remaining(), 0);

        // Six records including a discarded parent and the top node;
        // four retained edges; only the selected three-child list is copied.
        let work = Work::new(budget);
        let result = tree.finish_glr(&parser, roots, 3, &work);
        if succeeds {
            let tree = result.unwrap();
            let node = tree.root().children().next().unwrap();
            assert_eq!(node.term(), 3);
            assert_eq!(
                node.children().map(|n| n.range()).collect::<Vec<_>>(),
                [0..1, 1..2, 2..3]
            );
            assert_eq!(work.remaining(), 0);
        } else {
            assert_eq!(result.unwrap_err(), ParseError::LimitExceeded { offset: 3 });
        }
    }
}

#[test]
fn stage_charges_only_reachable_records_and_restores_the_live_arena() {
    let parser = fork_parser(1, 1, 0);
    let mut tree = TreeBuilder::for_glr(true, 0);
    let mut roots = Roots::default();
    tree.leaf(&mut roots, 2, 0..1);
    let mut discarded = roots;
    for _ in 0..10_000 {
        tree.leaf(&mut discarded, 3, 1..2);
    }

    // One root-list entry, two node visits, one visited edge, two copies
    // and one copied edge. No charge or allocation for discarded records.
    for budget in [6, 7] {
        let work = Work::new(budget);
        let staged = tree.with_staged_output(|tree| tree.finish_stage(&parser, roots, 1, &work));
        assert_eq!(tree.node_count(), 10_001);
        if budget == 6 {
            assert_eq!(staged.unwrap_err(), ParseError::LimitExceeded { offset: 1 });
        } else {
            let staged = staged.unwrap();
            assert_eq!(work.remaining(), 0);
            let mut children = staged.root().children();
            assert_eq!(children.next().unwrap().range(), 0..1);
            assert!(children.next().is_none());
        }
    }
    tree.leaf(&mut roots, 3, 1..2);
    let full = tree
        .finish_glr(&parser, roots, 2, &Work::new(20_000))
        .unwrap();
    assert_eq!(full.root().children().count(), 2);
}

#[test]
fn deferred_repeat_is_metered_and_flattened_in_staged_and_final_output() {
    let parser = fork_parser(1, 1, 0);
    let repeat = parser.eof;
    let mut tree = TreeBuilder::for_wast_recovery(repeat);
    let base = Roots::default();
    let mut roots = base;
    for at in 0..32 {
        tree.leaf(&mut roots, 2, at..at + 1);
    }
    let check = |tree: SyntaxTree| {
        assert!(
            tree.root()
                .children()
                .map(|node| node.range())
                .eq((0..32).map(|at| at..at + 1))
        );
    };
    for budget in [135, 136, 136] {
        let work = Work::new(budget);
        let result = tree.with_staged_output(|tree| {
            let mut staged = roots;
            tree.reduce(&mut staged, base, repeat, 0..32, 32, &work)?;
            tree.finish_stage(&parser, staged, 32, &work)
        });
        assert_eq!(tree.node_count(), 32);
        if budget == 135 {
            assert_eq!(
                result.unwrap_err(),
                ParseError::LimitExceeded { offset: 32 }
            );
        } else {
            check(result.unwrap());
            assert_eq!(work.remaining(), 0);
        }
    }
    tree.reduce(&mut roots, base, repeat, 0..32, 32, &Work::new(1))
        .unwrap();
    let work = Work::new(69);
    check(tree.finish_glr(&parser, roots, 32, &work).unwrap());
    assert_eq!(work.remaining(), 0);
}

#[test]
fn deferred_reduction_preserves_the_prefix_and_leaves_trailing_records_outside() {
    let parser = fork_parser(1, 1, 0);
    let mut tree = TreeBuilder::for_glr(true, parser.names.len());
    let mut roots = Roots::default();
    tree.leaf(&mut roots, 2, 0..1);
    let outer = roots;
    tree.leaf(&mut roots, 2, 1..2);
    let inner = roots;
    tree.error(&mut roots, inner.len, 2..3, None);
    tree.error(&mut roots, inner.len, 3..4, None);
    tree.error(&mut roots, inner.len, 4..4, None);
    tree.leaf(&mut roots, 2, 4..5);
    assert_eq!(
        tree.reduce(&mut roots, inner, 3, 2..4, 5, &Work::new(1)),
        Err(ParseError::LimitExceeded { offset: 5 }),
    );
    tree.reduce(&mut roots, inner, 3, 2..4, 5, &Work::new(2))
        .unwrap();
    tree.reduce(&mut roots, outer, 3, 1..5, 5, &Work::new(1))
        .unwrap();

    let tree = tree.finish_glr(&parser, roots, 5, &Work::new(100)).unwrap();
    let mut children = tree.root().children();
    assert_eq!(children.next().unwrap().range(), 0..1);
    let outer = children.next().unwrap();
    assert_eq!(outer.range(), 1..5);
    let children: Vec<_> = outer.children().collect();
    assert_eq!(
        children
            .iter()
            .map(|n| (n.term(), n.range()))
            .collect::<Vec<_>>(),
        [(2, 1..2), (3, 2..4), (2, 4..5)]
    );
    let error = children[1].children().next().unwrap();
    assert!(error.is_error());
    assert_eq!(error.range(), 2..4);
}

#[test]
fn pruned_glr_keeps_tied_branches_in_order_after_a_shift() {
    let frontier = EDITOR_FRONTIER as u16;
    assert!(
        fork_parser(frontier + 1, frontier, 0)
            .parse_glr_pruned(SOURCE)
            .is_ok()
    );
    let later = fork_parser(frontier + 1, frontier + 1, 0);
    assert!(later.parse_glr(SOURCE).is_ok());
    assert!(matches!(
        later.parse_glr_pruned(SOURCE),
        Err(ParseError::UnexpectedToken { offset: 5, .. })
    ));
}

#[test]
fn pruned_glr_bounds_live_forks_and_shares_work() {
    let cap = EDITOR_MAX_BRANCHES as u16;
    assert!(fork_parser(cap, 1, 0).parse_glr_pruned(SOURCE).is_ok());
    assert_eq!(
        fork_parser(cap + 1, 1, 0)
            .parse_glr_pruned(SOURCE)
            .unwrap_err(),
        ParseError::LimitExceeded { offset: 0 }
    );
    assert!(fork_parser(1, 1, 800).parse_glr_pruned(SOURCE).is_ok());
    assert_eq!(
        fork_parser(12, 1, 800)
            .parse_glr_pruned(SOURCE)
            .unwrap_err(),
        ParseError::LimitExceeded { offset: 5 }
    );
}
