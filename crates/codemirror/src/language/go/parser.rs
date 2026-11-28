use std::{borrow::Cow, sync::LazyLock};

use super::{
    super::lezer::{
        CLOSED_BY, GROUP, GrammarSpec, ISOLATE, Isolation, LANGUAGE_DATA, LanguageFacet, NodeTypes,
        OPENED_BY, ParseError, Parser, ReuseStats, SerializedTables, Specializer, SyntaxTree,
        TreeFragment,
    },
    GoNodeKind, Node, tables, tokens,
};

static PARSER: LazyLock<Parser> = LazyLock::new(|| {
    Parser::deserialize_with_hooks(
        GrammarSpec {
            tables: SerializedTables {
                version: 14,
                states: tables::STATES,
                state_data: tables::STATE_DATA,
                goto: tables::GOTO,
                token_data: tables::TOKEN_DATA,
            },
            node_names: tables::NODE_NAMES,
            max_term: tables::MAX_TERM,
            repeat_node_count: tables::REPEAT_NODE_COUNT,
            tokenizers: &tokens::tokenizers(),
            top_rule: tables::TOP_RULE,
            token_prec: tables::TOKEN_PREC,
        },
        &[Specializer::context_free(
            tokens::IDENTIFIER,
            tables::SPECIALIZATIONS,
        )],
    )
    .and_then(|parser| parser.with_dynamic_precedence(tables::DYNAMIC_PRECEDENCES))
    .expect("valid tables for the pinned Go grammar")
    .with_deterministic_context_tracker(tokens::track_tokens())
});

pub(crate) fn stock_data_facet() -> &'static LanguageFacet {
    static FACET: LazyLock<LanguageFacet> =
        LazyLock::new(|| LanguageFacet::new(Some(super::language_data())));
    &FACET
}

pub(crate) fn stock_node_types() -> &'static NodeTypes {
    static TYPES: LazyLock<NodeTypes> = LazyLock::new(|| {
        let top = tables::TOP_RULE[1];
        NodeTypes::new(
            tables::NODE_NAMES,
            tables::REPEAT_NODE_COUNT,
            top,
            tables::SKIPPED,
            |term, _, props| {
                if term == top {
                    props.insert(&LANGUAGE_DATA, Some(stock_data_facet().clone()));
                }
                let group = if tables::EXPRESSIONS.contains(&term) {
                    Some("Expr")
                } else if tables::TYPES.contains(&term) {
                    Some("Type")
                } else if tables::STATEMENTS.contains(&term) {
                    Some("Statement")
                } else {
                    None
                };
                if let Some(group) = group {
                    props.insert(&GROUP, vec![group.into()]);
                }
                match term {
                    2 | 12 | 20 => props.insert(&ISOLATE, Some(Isolation::Auto)),
                    13 => props.insert(&OPENED_BY, vec!["(".into()]),
                    14 => props.insert(&CLOSED_BY, vec![")".into()]),
                    25 => props.insert(&OPENED_BY, vec!["{".into()]),
                    26 => props.insert(&CLOSED_BY, vec!["}".into()]),
                    53 => props.insert(&OPENED_BY, vec!["[".into()]),
                    54 => props.insert(&CLOSED_BY, vec!["]".into()]),
                    _ => {}
                }
            },
        )
    });
    &TYPES
}

pub(crate) fn parse_raw(source: &str, strict: bool) -> Result<SyntaxTree, ParseError> {
    if strict {
        return PARSER.parse_editor_strict(source);
    }
    match parse_tree(source) {
        Err(ParseError::UnexpectedToken { .. }) => PARSER.parse_recovering(source),
        result => result,
    }
}

pub(super) fn parse_editor_strict(source: &str) -> Result<Vec<Node>, ParseError> {
    let input = Input::new(source);
    let tree = PARSER
        .parse_editor_strict(&input.text)
        .map_err(|error| error.map_offset(|at| input.original_byte(at)))?;
    Ok(nodes(&tree, input.text.len(), &input))
}

#[derive(Debug)]
pub(super) struct Cache {
    tree: SyntaxTree,
    pub(super) reuse: ReuseStats,
}

pub(super) fn parse(
    source: &str,
    prefix: Option<(&Cache, usize)>,
) -> (Vec<Node>, bool, Option<Cache>) {
    let input = Input::new(source);
    let fragment = prefix.map(|(cache, end)| {
        let end = input.prefix_end(source, end);
        TreeFragment {
            tree: &cache.tree,
            old_range: 0..end,
            new_range: 0..end,
        }
    });
    let text = &input.text;
    let strict = PARSER.parse_incremental(text, fragment.clone());
    let (failure, ambiguous) = match strict {
        Ok(tree) => return published(tree, &input, false),
        Err(ParseError::Ambiguous { .. }) => match PARSER.parse_glr(text) {
            Ok(tree) => return published(tree, &input, false),
            Err(error) => (error, true),
        },
        Err(error) => (error, false),
    };

    // Do not retry a strict parse that hit its work or progress limit.
    if let ParseError::UnexpectedToken { offset, .. } = failure {
        let recovered = if ambiguous {
            PARSER.parse_recovering(text)
        } else {
            // Repeat deterministic LR to retain its checked prefix seed.
            PARSER.parse_incremental_recovering(text, fragment.as_slice())
        };
        if let Ok(tree) = recovered {
            return published(tree, &input, true);
        }

        // Bounded recovery did not finish. Keep the previous Go fallback:
        // try missing closers at EOF, then before the line with the error.
        let (last_end, suffix) = recovery_delimiters(text);
        // A specialization can leave a statement without its final
        // semicolon. Try an explicit one at EOF if no delimiter is missing.
        let end_suffix = if suffix.is_empty() { ";" } else { &suffix };
        if offset >= last_end
            && let Some(tree) = close_prefix(text, end_suffix)
        {
            return (nodes(&tree, text.len(), &input), true, None);
        }
        if let Some(at) = text.as_bytes()[..offset.min(text.len().saturating_sub(1))]
            .iter()
            .rposition(|&byte| byte == b'\n')
            .map(|at| at + 1)
        {
            let (_, suffix) = recovery_delimiters(&text[..at]);
            if let Some(tree) = close_prefix(&text[..at], &suffix) {
                return (nodes(&tree, at, &input), true, None);
            }
        }
    }
    let mut root = vec![Node {
        term: tables::TOP_RULE[1],
        range: 0..source.len(),
        parent: None,
        children: Vec::new(),
    }];
    error_node(&mut root, 0, 0..source.len());
    (root, true, None)
}

fn published(
    tree: SyntaxTree,
    input: &Input<'_>,
    recovered: bool,
) -> (Vec<Node>, bool, Option<Cache>) {
    let nodes = nodes(&tree, input.text.len(), input);
    let mut reuse = tree.reuse_stats();
    reuse.resumed_at = input.original_byte(reuse.resumed_at);
    let cache = tree
        .has_incremental_state()
        .then_some(Cache { tree, reuse });
    (nodes, recovered, cache)
}

struct Input<'a> {
    text: Cow<'a, str>,
    original_len: usize,
    // Starts of CRLF pairs in the original and normalized text.
    breaks: Vec<(usize, usize)>,
}

impl<'a> Input<'a> {
    fn new(source: &'a str) -> Self {
        let breaks = source
            .match_indices("\r\n")
            .enumerate()
            .map(|(removed, (at, _))| (at, at - removed))
            .collect();
        // CodeMirror parses with LF. Keep the original text in the document.
        let text = if source.contains('\r') {
            Cow::Owned(source.replace("\r\n", "\n").replace('\r', "\n"))
        } else {
            Cow::Borrowed(source)
        };
        Self {
            text,
            original_len: source.len(),
            breaks,
        }
    }

    fn original_byte(&self, at: usize) -> usize {
        at + self.breaks.partition_point(|&(_, lf)| lf < at)
    }

    fn prefix_end(&self, source: &str, mut end: usize) -> usize {
        // Stop before a CR that the edit could join to or split from an LF.
        if end > 0 && source.as_bytes()[end - 1] == b'\r' {
            end -= 1;
        }
        end - self.breaks.partition_point(|&(cr, _)| cr < end)
    }
}

fn recovery_delimiters(source: &str) -> (usize, String) {
    tokens::recovery_delimiters(PARSER.tables().token_data(), source)
}

fn parse_tree(source: &str) -> Result<SyntaxTree, ParseError> {
    PARSER.parse(source).or_else(|error| try_glr(source, error))
}

fn try_glr(source: &str, error: ParseError) -> Result<SyntaxTree, ParseError> {
    match error {
        ParseError::Ambiguous { .. } => PARSER.parse_glr(source),
        _ => Err(error),
    }
}

fn close_prefix(source: &str, suffix: &str) -> Option<SyntaxTree> {
    if suffix.is_empty() {
        return parse_tree(source).ok();
    }
    let prefix = if suffix.starts_with("*/") {
        format!("{source}{suffix}")
    } else {
        // A line comment must not consume a synthetic delimiter.
        format!("{source}\n{suffix}")
    };
    parse_tree(&prefix).ok()
}

fn nodes(tree: &SyntaxTree, cutoff: usize, input: &Input<'_>) -> Vec<Node> {
    let mut stack = vec![(tree.root(), None::<usize>)];
    let mut nodes: Vec<Node> = Vec::new();
    while let Some((syntax, parent)) = stack.pop() {
        let id = nodes.len();
        let range = syntax.range();
        if let Some(parent) = parent {
            nodes[parent].children.push(id);
        }
        let missing = range.start >= cutoff && range.end > cutoff;
        let start = input.original_byte(range.start.min(cutoff));
        let node_end = input.original_byte(range.end.min(cutoff));
        nodes.push(Node {
            term: if missing { 0 } else { syntax.term() },
            range: start..node_end,
            parent,
            children: Vec::new(),
        });
        if !missing {
            if range.end > cutoff
                && GoNodeKind::from_term(syntax.term()) == GoNodeKind::BlockComment
            {
                // Block comment delimiters are anonymous in the grammar.
                error_node(&mut nodes, id, node_end..node_end);
            }
            stack.extend(syntax.children().rev().map(|child| (child, Some(id))));
        }
    }
    let parsed_end = input.original_byte(cutoff);
    let end = input.original_len;
    nodes[0].range = 0..end;
    if parsed_end < end {
        error_node(&mut nodes, 0, parsed_end..end);
    }
    nodes
}

fn error_node(nodes: &mut Vec<Node>, parent: usize, range: std::ops::Range<usize>) {
    let id = nodes.len();
    nodes[parent].children.push(id);
    nodes.push(Node {
        term: 0,
        range,
        parent: Some(parent),
        children: Vec::new(),
    });
}
