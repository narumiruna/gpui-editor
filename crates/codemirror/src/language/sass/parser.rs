use std::{ops::Range, sync::LazyLock};

use crate::language::css::input::Input;

use super::{
    super::lezer::{GrammarSpec, ParseError, Parser, SerializedTables, Specializer, SyntaxTree},
    Node, SassDialect, tables, tokens,
};

static SCSS: LazyLock<Parser> = LazyLock::new(|| parser(false));
static INDENTED: LazyLock<Parser> = LazyLock::new(|| parser(true));
static NAMES: LazyLock<Vec<&str>> = LazyLock::new(|| tables::NODE_NAMES.split(' ').collect());

const MAX_PARSE_BYTES: usize = 1 << 20;
const MAX_REPAIR_BYTES: usize = 64 * 1024;

pub(super) fn name(term: u16) -> &'static str {
    NAMES[usize::from(term)]
}

fn parser(indented: bool) -> Parser {
    let parser = Parser::deserialize_with_hooks(
        GrammarSpec {
            tables: SerializedTables {
                version: tables::VERSION,
                states: tables::STATES,
                state_data: tables::STATE_DATA,
                goto: tables::GOTO,
                token_data: tables::TOKEN_DATA,
            },
            node_names: tables::NODE_NAMES,
            max_term: tables::MAX_TERM,
            repeat_node_count: tables::REPEAT_NODE_COUNT,
            tokenizers: &tokens::tokenizers(indented),
            top_rule: tables::TOP_RULE,
            token_prec: tables::TOKEN_PREC,
        },
        &[
            Specializer::context_free(tables::IDENTIFIER, tables::IDENTIFIERS),
            Specializer::context_free(tables::CALLEE, tables::CALLEES),
            Specializer::context_free(tables::AT_KEYWORD, tables::AT_KEYWORDS),
            Specializer::context_free(tables::QUERY_IDENTIFIER, tables::QUERY_IDENTIFIERS),
        ],
    )
    .expect("valid tables for the pinned Sass grammar");
    // In the published grammar the indented dialect is selected by the
    // external tokenizers. The DFA, grammar states, and start rule are shared.
    if indented {
        parser.with_deterministic_context_tracker(tokens::tracker())
    } else {
        parser
    }
}

pub(super) fn parse(
    source: &str,
    dialect: SassDialect,
    prefix: Option<(&SyntaxTree, usize)>,
) -> (Vec<Node>, Option<SyntaxTree>) {
    if source.len() > MAX_PARSE_BYTES {
        return (materialize(None, 0, source.len(), |at| at), None);
    }
    let input = Input::new(source, true);
    let fragment = prefix.map(|(tree, end)| input.fragment(tree, end));
    let byte = |at| input.byte(at);
    let parser = match dialect {
        SassDialect::Scss => &*SCSS,
        SassDialect::Indented => &*INDENTED,
    };
    let parse = |text: &str| {
        parser.parse(text).or_else(|error| match error {
            ParseError::Ambiguous { .. } => parser.parse_glr(text),
            _ => Err(error),
        })
    };
    let logical = input.text.as_ref();
    let result = match parser.parse_incremental(logical, fragment) {
        Ok(tree) => {
            return (
                materialize(Some(&tree), logical.len(), source.len(), byte),
                Some(tree),
            );
        }
        Err(ParseError::Ambiguous { .. }) => parser.parse_glr(logical),
        Err(error) => Err(error),
    };
    let (tree, cutoff) = match result {
        Ok(tree) => (Some(tree), logical.len()),
        Err(ParseError::UnexpectedToken { offset, .. }) if logical.len() <= MAX_REPAIR_BYTES => {
            recover(logical, offset, dialect, parse)
        }
        Err(_) => (None, 0),
    };
    (materialize(tree.as_ref(), cutoff, source.len(), byte), None)
}

// Bounded local repair. Only append to a prefix, so no real source is removed
// from a valid syntax span and synthetic text cannot leak into the tree.
fn recover(
    input: &str,
    offset: usize,
    dialect: SassDialect,
    parse: impl Fn(&str) -> Result<SyntaxTree, ParseError>,
) -> (Option<SyntaxTree>, usize) {
    let mut sites = Vec::new();
    if offset == input.len() {
        sites.push(input.len());
    }
    sites.extend(
        input[..offset]
            .match_indices(['\n', ';', '}'])
            .rev()
            .take(3)
            .map(|(at, _)| at + 1),
    );
    sites.dedup();
    for cutoff in sites {
        let prefix = &input[..cutoff];
        if cutoff < input.len()
            && let Ok(tree) = parse(prefix)
        {
            return (Some(tree), cutoff);
        }
        let ends = missing_closers(prefix);
        let fillings: &[&str] = match dialect {
            SassDialect::Scss => &["", ";", "0", ":0", "{}"],
            SassDialect::Indented => &["", "0", ":0", "\n  x: 0"],
        };
        for fill in fillings {
            let mut fixed = String::with_capacity(cutoff + fill.len() + ends.len() + 1);
            fixed.push_str(prefix);
            fixed.push_str(fill);
            fixed.push_str(&ends);
            if fixed.len() > cutoff
                && let Ok(tree) = parse(&fixed)
            {
                return (Some(tree), cutoff);
            }
            if dialect == SassDialect::Scss {
                fixed.push(';');
                if let Ok(tree) = parse(&fixed) {
                    return (Some(tree), cutoff);
                }
            }
        }
    }
    (None, 0)
}

fn missing_closers(source: &str) -> String {
    let mut ends = Vec::new();
    let mut quote = None;
    let mut bytes = source.bytes().peekable();
    while let Some(ch) = bytes.next() {
        if let Some(open) = quote {
            if ch == open || ch == b'\n' {
                quote = None;
            } else if ch == b'\\' {
                bytes.next();
            }
            continue;
        }
        match ch {
            b'\'' | b'"' => quote = Some(ch),
            b'/' if bytes.next_if_eq(&b'/').is_some() => {
                while bytes.next_if(|&ch| ch != b'\n').is_some() {}
            }
            b'/' if bytes.next_if_eq(&b'*').is_some() => {
                while let Some(ch) = bytes.next() {
                    if ch == b'*' && bytes.next_if_eq(&b'/').is_some() {
                        break;
                    }
                }
            }
            b'(' => ends.push(b')'),
            b'[' => ends.push(b']'),
            b'{' => ends.push(b'}'),
            b')' | b']' | b'}' if ends.last() == Some(&ch) => {
                ends.pop();
            }
            _ => {}
        }
    }
    // A quote, if open at EOF, closes before surrounding delimiters.
    quote
        .into_iter()
        .chain(ends.into_iter().rev())
        .map(char::from)
        .collect()
}

fn materialize(
    tree: Option<&SyntaxTree>,
    cutoff: usize,
    len: usize,
    byte: impl Fn(usize) -> usize,
) -> Vec<Node> {
    let mut nodes = Vec::<Node>::new();
    if let Some(tree) = tree {
        let repaired = cutoff < tree.root().range().end;
        let mut pending = vec![(Some(tree.root()), None::<usize>)];
        while let Some((syntax, parent)) = pending.pop() {
            let Some(syntax) = syntax else {
                let at = byte(cutoff);
                push(&mut nodes, 0, at..at, parent);
                continue;
            };
            let range = syntax.range();
            if repaired && range.start >= cutoff && range.end > cutoff && parent.is_some() {
                continue;
            }
            let clipped = range.end > cutoff;
            let id = push(
                &mut nodes,
                syntax.term(),
                byte(range.start.min(cutoff))..byte(range.end.min(cutoff)),
                parent,
            );
            let missing_closer = syntax.children().next_back().is_some_and(|last| {
                let r = last.range();
                r.start >= cutoff
                    && r.end > cutoff
                    && matches!(name(last.term()), "}" | "]" | ")" | "InterpolationEnd")
            });
            if clipped
                && (missing_closer
                    || !syntax.children().any(|child| {
                        let r = child.range();
                        r.start < cutoff && r.end > cutoff
                    }))
            {
                pending.push((None, Some(id)));
            }
            pending.extend(syntax.children().rev().map(|child| (Some(child), Some(id))));
        }
    } else {
        push(&mut nodes, tables::TOP_RULE[1], 0..len, None);
    }
    nodes[0].range = 0..len;
    let end = byte(cutoff);
    if end < len {
        push(&mut nodes, 0, end..len, Some(0));
    }
    nodes
}

fn push(nodes: &mut Vec<Node>, term: u16, range: Range<usize>, parent: Option<usize>) -> usize {
    let id = nodes.len();
    if let Some(parent) = parent {
        nodes[parent].children.push(id);
    }
    nodes.push(Node {
        term,
        range,
        parent,
        children: Vec::new(),
    });
    id
}
