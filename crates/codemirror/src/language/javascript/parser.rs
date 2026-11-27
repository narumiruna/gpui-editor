use std::{borrow::Cow, sync::LazyLock};

use super::{
    super::lezer::{
        GrammarSpec, ParseError, Parser, SerializedTables, Specialization, Specializer, SyntaxTree,
    },
    JavascriptNodeKind, JavascriptOptions, Node, specialize, tables, tokens,
};

static PARSERS: [LazyLock<Parser>; 4] = [
    LazyLock::new(|| parser(false, false, tables::TOP_RULE)),
    LazyLock::new(|| parser(false, true, tables::TOP_RULE)),
    LazyLock::new(|| parser(true, false, tables::TOP_RULE)),
    LazyLock::new(|| parser(true, true, tables::TOP_RULE)),
];
static EXPRESSIONS: [LazyLock<Parser>; 4] = [
    LazyLock::new(|| parser(false, false, tables::SINGLE_EXPRESSION)),
    LazyLock::new(|| parser(false, true, tables::SINGLE_EXPRESSION)),
    LazyLock::new(|| parser(true, false, tables::SINGLE_EXPRESSION)),
    LazyLock::new(|| parser(true, true, tables::SINGLE_EXPRESSION)),
];
static NAMES: LazyLock<Vec<&'static str>> =
    LazyLock::new(|| tables::NODE_NAMES.split(' ').collect());

pub(super) fn name(term: u16) -> &'static str {
    NAMES[usize::from(term)]
}

fn parser(typescript: bool, jsx: bool, top_rule: [u16; 2]) -> Parser {
    let token_data = if typescript {
        Cow::Borrowed(tables::TS_TOKEN_DATA)
    } else {
        let mut patched = tables::TS_TOKEN_DATA.to_owned();
        patched.replace_range(tables::JS_DFA_PATCH_RANGE, tables::JS_DFA_PATCH);
        Cow::Owned(patched)
    };
    let specializer = |term, table: &[(&str, u16)]| {
        let entries: Vec<_> = table
            .iter()
            .filter_map(|&(text, entry)| {
                let term = entry >> 1;
                (typescript || !tables::TS_TERMS.contains(&term)).then_some((
                    text,
                    if entry & 1 == 0 {
                        Specialization::Replace(term)
                    } else {
                        Specialization::Extend(term)
                    },
                ))
            })
            .collect();
        Specializer::context_free(term, &entries)
    };
    Parser::deserialize_with_hooks(
        GrammarSpec {
            tables: SerializedTables {
                version: 14,
                states: tables::STATES,
                state_data: tables::STATE_DATA,
                goto: tables::GOTO,
                token_data: &token_data,
            },
            node_names: tables::NODE_NAMES,
            max_term: tables::MAX_TERM,
            repeat_node_count: tables::REPEAT_NODE_COUNT,
            tokenizers: &tokens::tokenizers(jsx),
            top_rule,
            token_prec: tables::TOKEN_PREC,
        },
        &[
            specializer(328, specialize::IDENTIFIER),
            specializer(344, specialize::WORD),
            specializer(95, specialize::LESS_THAN),
        ],
    )
    .and_then(|parser| parser.with_dynamic_precedence(tables::DYNAMIC_PRECEDENCE))
    .expect("valid tables for the pinned JavaScript grammar")
    .with_deterministic_context_tracker(tokens::track_newline())
}

pub(super) fn configured(options: JavascriptOptions, expression: bool) -> &'static Parser {
    let variants = if expression { &EXPRESSIONS } else { &PARSERS };
    &variants[usize::from(options.typescript) * 2 + usize::from(options.jsx)]
}

pub(super) fn parse_fresh(parser: &Parser, source: &str) -> Result<SyntaxTree, ParseError> {
    parser
        .parse(source)
        .or_else(|error| try_glr(parser, source, error))
}

pub(super) fn try_glr(
    parser: &Parser,
    source: &str,
    error: ParseError,
) -> Result<SyntaxTree, ParseError> {
    match error {
        ParseError::Ambiguous { .. } => parser.parse_glr(source),
        _ => Err(error),
    }
}

pub(super) fn collect(tree: &SyntaxTree, end: usize) -> Vec<Node> {
    let truncated = end < tree.root().range().end;
    let mut pending = vec![(Some(tree.root()), None::<usize>)];
    let mut nodes: Vec<Node> = Vec::new();
    while let Some((syntax, parent)) = pending.pop() {
        let id = nodes.len();
        if let Some(parent) = parent {
            nodes[parent].children.push(id);
        }
        let Some(syntax) = syntax else {
            nodes.push(Node {
                kind: JavascriptNodeKind("⚠"),
                term: Some(0),
                range: end..end,
                parent,
                children: Vec::new(),
            });
            continue;
        };
        let term = syntax.term();
        let original = syntax.range();
        let range = original.start..original.end.min(end);
        nodes.push(Node {
            kind: JavascriptNodeKind(name(term)),
            term: Some(term),
            range,
            parent,
            children: Vec::new(),
        });
        if original.end > end
            && syntax
                .children()
                .next_back()
                .is_none_or(|child| child.range().start >= end || child.range().end <= end)
        {
            pending.push((None, Some(id)));
        }
        pending.extend(
            syntax
                .children()
                .rev()
                .filter(|child| !truncated || child.range().start < end)
                .map(|child| (Some(child), Some(id))),
        );
    }
    nodes
}
