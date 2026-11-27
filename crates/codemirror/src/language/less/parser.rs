use std::{ops::Range, sync::LazyLock};

use crate::language::{
    css::input::Input,
    lezer::{
        ExternalOptions, ExternalToken, ExternalTokenizer, GrammarSpec, ParseError, Parser,
        SerializedTables,
        Specialization::{Extend, Replace},
        Specializer, StackView, SyntaxNode, SyntaxTree, TokenInput, Tokenizer,
    },
};

use super::{
    LessNodeKind::{self, *},
    Node,
    recovery::{self, Repair},
    tables,
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
            max_term: 133,
            repeat_node_count: 10,
            tokenizers: &[
                external(descendant),
                external(unit),
                external(arg_list),
                Tokenizer::Internal(0),
                Tokenizer::Internal(1),
                Tokenizer::Internal(2),
                Tokenizer::Internal(3),
                Tokenizer::Internal(4),
            ],
            top_rule: [0, 5],
            token_prec: 2180,
        },
        &[
            Specializer::context_free(
                116,
                &[
                    ("lang", Replace(20)),
                    ("nth-child", Replace(20)),
                    ("nth-last-child", Replace(20)),
                    ("nth-of-type", Replace(20)),
                    ("nth-last-of-type", Replace(20)),
                    ("dir", Replace(20)),
                    ("host-context", Replace(20)),
                    ("and", Replace(122)),
                    ("or", Replace(122)),
                    ("not", Replace(37)),
                    ("only", Replace(37)),
                    ("url", Replace(43)),
                    ("url-prefix", Replace(43)),
                    ("domain", Replace(43)),
                    ("regexp", Replace(43)),
                    ("when", Extend(58)),
                    ("selector", Replace(71)),
                    ("from", Replace(86)),
                    ("to", Replace(87)),
                ],
            ),
            Specializer::context_free(
                23,
                &[
                    ("@import", Replace(63)),
                    ("@plugin", Replace(63)),
                    ("@media", Replace(76)),
                    ("@charset", Replace(78)),
                    ("@namespace", Replace(80)),
                    ("@keyframes", Replace(83)),
                    ("@supports", Replace(89)),
                ],
            ),
        ],
    )
    .expect("verified Less grammar tables")
});

type ScanResult = Result<Option<ExternalToken>, ParseError>;

fn external(scan: fn(&mut TokenInput<'_>, &StackView<'_>) -> ScanResult) -> Tokenizer {
    Tokenizer::External(ExternalTokenizer::new_deterministic(
        ExternalOptions::default(),
        scan,
    ))
}

// The external tokenizers use the upstream's UTF-16 code unit predicates.
fn alpha(unit: u16) -> bool {
    matches!(unit, 65..=90 | 97..=122 | 161..)
}

fn space(unit: u16) -> bool {
    matches!(unit, 9..=13 | 32 | 133 | 160 | 5760 | 8192..=8202 | 8232 | 8233 | 8239 | 8287 | 12288)
}

fn descendant(input: &mut TokenInput<'_>, _: &StackView<'_>) -> ScanResult {
    if !input.peek(-1)?.is_some_and(space) {
        return Ok(None);
    }
    Ok(input
        .peek(0)?
        .is_some_and(|unit| alpha(unit) || matches!(unit, 95 | 35 | 46 | 91 | 58 | 45))
        .then(|| input.token(110)))
}

fn unit(input: &mut TokenInput<'_>, _: &StackView<'_>) -> ScanResult {
    if input.peek(-1)?.is_some_and(space) {
        return Ok(None);
    }
    let next = input.peek(0)?;
    if next == Some(37) {
        input.advance(1)?;
    } else if next.is_some_and(alpha) {
        while input.advance(1)?.is_some_and(alpha) {}
    } else {
        return Ok(None);
    }
    Ok(Some(input.token(1)))
}

fn arg_list(input: &mut TokenInput<'_>, _: &StackView<'_>) -> ScanResult {
    if input.peek(0)? == Some(40)
        && input
            .peek(-1)?
            .is_some_and(|unit| alpha(unit) || matches!(unit, 48..=57 | 95 | 45))
    {
        input.advance(1)?;
        return Ok(Some(input.token(2)));
    }
    Ok(None)
}

pub(super) fn parse_tree(source: &str) -> Result<SyntaxTree, ParseError> {
    PARSER.parse(source).or_else(|error| match error {
        ParseError::Ambiguous { .. } => PARSER.parse_glr(source),
        error => Err(error),
    })
}

pub(super) fn parse(
    source: &str,
    prefix: Option<(&SyntaxTree, usize)>,
) -> (Vec<Node>, Option<SyntaxTree>) {
    let input = Input::new(source, false);
    let fragment = prefix.map(|(tree, end)| input.fragment(tree, end));
    let byte = |at| input.byte(at);
    let result = match PARSER.parse_incremental(&input.text, fragment) {
        Ok(tree) => return (flatten(&tree, None, byte), Some(tree)),
        Err(ParseError::Ambiguous { .. }) => PARSER.parse_glr(&input.text),
        Err(error) => Err(error),
    };
    let nodes = match result {
        Ok(tree) => flatten(&tree, None, byte),
        Err(error) => match recovery::repair(&input.text, error) {
            Some((tree, repair)) => flatten(&tree, Some(&repair), byte),
            None => vec![
                Node {
                    kind: StyleSheet,
                    range: 0..source.len(),
                    parent: None,
                    children: vec![1],
                    closing: None,
                },
                Node {
                    kind: Error,
                    range: 0..source.len(),
                    parent: Some(0),
                    children: Vec::new(),
                    closing: None,
                },
            ],
        },
    };
    (nodes, None)
}

enum Entry<'a> {
    Syntax(SyntaxNode<'a>),
    Error(Range<usize>),
}

fn flatten(
    tree: &SyntaxTree,
    repair: Option<&Repair<'_>>,
    byte: impl Fn(usize) -> usize,
) -> Vec<Node> {
    let mut pending = vec![(Entry::Syntax(tree.root()), None::<usize>)];
    let mut nodes: Vec<Node> = Vec::new();
    while let Some((entry, parent)) = pending.pop() {
        let id = nodes.len();
        let (range, kind, closing) = match entry {
            Entry::Error(range) => (range, Error, None),
            Entry::Syntax(syntax) => {
                let raw = syntax.range();
                let had_width = !raw.is_empty();
                let range = match repair {
                    Some(repair) => repair.map_range(raw),
                    None => raw,
                };
                if parent.is_some() && range.is_empty() && had_width {
                    (range, Error, None)
                } else {
                    push_children(&mut pending, syntax, id, &range, repair);
                    let closing = match syntax.term() {
                        26 => Some(CloseParen),
                        60 => Some(CloseBrace),
                        _ => None,
                    };
                    (range, LessNodeKind::from_name(syntax.name()), closing)
                }
            }
        };
        let range = byte(range.start)..byte(range.end);
        if let Some(parent) = parent {
            if kind == Error
                && range.is_empty()
                && nodes[parent]
                    .children
                    .last()
                    .is_some_and(|&child| nodes[child].kind == Error && nodes[child].range == range)
            {
                continue;
            }
            nodes[parent].children.push(id);
        }
        nodes.push(Node {
            kind,
            range,
            parent,
            children: Vec::new(),
            closing,
        });
    }
    nodes
}

fn push_children<'a>(
    pending: &mut Vec<(Entry<'a>, Option<usize>)>,
    syntax: SyntaxNode<'a>,
    parent: usize,
    range: &Range<usize>,
    repair: Option<&Repair<'_>>,
) {
    let errors = repair.map_or(&[][..], |repair| repair.errors_in(range));
    if errors.is_empty() || syntax.children().len() == 0 && syntax.name() != "StyleSheet" {
        pending.extend(
            syntax
                .children()
                .rev()
                .map(|node| (Entry::Syntax(node), Some(parent))),
        );
        return;
    }
    let repair = repair.expect("removed ranges belong to a repair");
    let mut errors = errors.iter().rev().peekable();
    for child in syntax.children().rev() {
        let range = repair.map_range(child.range());
        while errors.peek().is_some_and(|error| {
            error.start >= range.end && !(range.is_empty() && error.start == range.start)
        }) {
            pending.push((Entry::Error(errors.next().unwrap().clone()), Some(parent)));
        }
        // A token may absorb masked text (for example as literal content).
        // Keep its real token range rather than adding an overlapping error.
        while errors
            .peek()
            .is_some_and(|error| error.end > range.start && !range.is_empty())
        {
            errors.next();
        }
        pending.push((Entry::Syntax(child), Some(parent)));
    }
    pending.extend(errors.map(|error| (Entry::Error(error.clone()), Some(parent))));
}
