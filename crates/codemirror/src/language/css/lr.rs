use std::{
    collections::HashSet,
    fmt,
    sync::{Arc, LazyLock},
};

use super::{
    super::lezer::{ParseError, Parser, ReuseStats, Specialization, Specializer, SyntaxTree},
    CssNodeKind, Node,
    input::Input,
    tables, tokens,
};

const HASH: u16 = 172;
const COLOR: Specialization = Specialization::Replace(152);

#[derive(Clone, Copy, Debug)]
pub(super) enum Mode {
    Sheet,
    Styles,
}

impl Mode {
    fn parser(self, color: Specializer) -> Parser {
        let top = match self {
            Self::Sheet => [0, 6],
            Self::Styles => [1, 129],
        };
        tables::parser(top, color)
    }

    fn no_colors(self) -> &'static Parser {
        static SHEET: LazyLock<Parser> =
            LazyLock::new(|| Mode::Sheet.parser(Specializer::context_free(HASH, &[])));
        static STYLES: LazyLock<Parser> =
            LazyLock::new(|| Mode::Styles.parser(Specializer::context_free(HASH, &[])));
        match self {
            Self::Sheet => &SHEET,
            Self::Styles => &STYLES,
        }
    }
}

#[derive(Debug)]
pub(super) struct Strict {
    tree: SyntaxTree,
    colors: Option<Arc<ColorParser>>,
}

impl Strict {
    pub(super) fn reuse_stats(&self) -> ReuseStats {
        self.tree.reuse_stats()
    }
}

struct ColorParser {
    parser: Parser,
    spellings: HashSet<Box<str>>,
}

impl fmt::Debug for ColorParser {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ColorParser")
            .field("spellings", &self.spellings.len())
            .finish_non_exhaustive()
    }
}

pub(super) fn parse_incremental(
    source: &str,
    mode: Mode,
    prefix: Option<(&Strict, usize)>,
) -> Result<(Vec<Node>, Option<Strict>), ParseError> {
    let input = Input::new(source, false);
    // The color callback only replaces exact hex spellings of these lengths.
    // Include all potential matches; an entry cannot affect a longer hash token.
    let logical = input.text.as_ref();
    let spellings: HashSet<_> = logical
        .match_indices('#')
        .flat_map(|(at, _)| {
            tokens::COLOR_LENGTHS.into_iter().filter_map(move |len| {
                logical
                    .get(at..at + len)
                    .filter(|text| tokens::is_color(text))
            })
        })
        .collect();
    // Keep the parser identity only if its immutable table covers the new
    // input. A new spelling needs a new parser and a fresh checkpoint history.
    let colors = if let Some(old) = prefix.and_then(|(strict, _)| strict.colors.as_ref())
        && spellings.iter().all(|&text| old.spellings.contains(text))
    {
        Some(Arc::clone(old))
    } else if spellings.is_empty() {
        None
    } else {
        let entries: Vec<_> = spellings.iter().map(|&text| (text, COLOR)).collect();
        Some(Arc::new(ColorParser {
            parser: mode.parser(Specializer::context_free(HASH, &entries)),
            spellings: spellings.into_iter().map(Into::into).collect(),
        }))
    };
    let parser = colors
        .as_ref()
        .map_or_else(|| mode.no_colors(), |colors| &colors.parser);
    let fragment = prefix.map(|(strict, end)| input.fragment(&strict.tree, end));
    match parser.parse_incremental(&input.text, fragment) {
        Ok(tree) => Ok((flatten(&tree, &input), Some(Strict { tree, colors }))),
        Err(ParseError::Ambiguous { .. }) => {
            Ok((flatten(&parser.parse_glr(&input.text)?, &input), None))
        }
        Err(error) => Err(error),
    }
}

// Recovery parses temporary inputs without reading or collecting checkpoints.
pub(super) fn parse(source: &str) -> Result<Vec<Node>, ParseError> {
    static SHEET: LazyLock<Parser> = LazyLock::new(|| Mode::Sheet.parser(color_callback()));
    parse_with(&SHEET, source)
}

pub(super) fn parse_styles(source: &str) -> Result<Vec<Node>, ParseError> {
    static STYLES: LazyLock<Parser> = LazyLock::new(|| Mode::Styles.parser(color_callback()));
    parse_with(&STYLES, source)
}

fn color_callback() -> Specializer {
    Specializer::new(HASH, |text, _| Ok(tokens::is_color(text).then_some(COLOR)))
}

fn parse_with(parser: &Parser, source: &str) -> Result<Vec<Node>, ParseError> {
    let input = Input::new(source, false);
    let tree = parser.parse(&input.text).or_else(|error| match error {
        ParseError::Ambiguous { .. } => parser.parse_glr(&input.text),
        other => Err(other),
    })?;
    Ok(flatten(&tree, &input))
}

fn flatten(tree: &SyntaxTree, input: &Input<'_>) -> Vec<Node> {
    let mut stack = vec![(tree.root(), None::<usize>)];
    let mut nodes: Vec<Node> = Vec::new();
    while let Some((syntax, parent)) = stack.pop() {
        let id = nodes.len();
        let range = syntax.range();
        if let Some(parent) = parent {
            nodes[parent].children.push(id);
        }
        nodes.push(Node {
            kind: CssNodeKind::from_name(syntax.name()),
            term: Some(syntax.term()),
            range: input.byte(range.start)..input.byte(range.end),
            parent,
            children: Vec::new(),
        });
        stack.extend(syntax.children().rev().map(|child| (child, Some(id))));
    }
    nodes
}

#[cfg(test)]
mod tests {
    #[test]
    fn published_tables_use_local_comment_and_external_color_hooks() {
        use super::CssNodeKind::*;
        let source = "/*x\r\n😀*/\r\n@media (width >= 12px) {} @charset \"UTF-8\";";
        let nodes = super::parse(source).unwrap();
        let tagged: Vec<_> = nodes
            .iter()
            .filter(|node| {
                matches!(
                    node.kind,
                    Comment | Media | FeatureName | Unit | CompareOp | Charset
                )
            })
            .map(|node| (node.kind, &source[node.range.clone()]))
            .collect();
        assert_eq!(
            tagged,
            [
                (Comment, "/*x\r\n😀*/"),
                (Media, "@media"),
                (FeatureName, "width"),
                (CompareOp, ">="),
                (Unit, "px"),
                (Charset, "@charset"),
            ]
        );
        let source = "a { --color: #ffe; }";
        let nodes = super::parse(source).unwrap();
        assert!(
            nodes
                .iter()
                .any(|node| node.kind == ColorLiteral && &source[node.range.clone()] == "#ffe")
        );
    }
}
