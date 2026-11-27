use std::{ops::Range, sync::LazyLock};

use super::{
    super::{
        css::input::Input,
        lezer::{ParseError, Parser, SyntaxNode, SyntaxTree},
    },
    LiquidNodeKind::{self, *},
    Node, recovery, tables, tokens,
};

static PARSER: LazyLock<Parser> = LazyLock::new(|| tables::parser(tokens::tokenizers()));

pub(super) fn parse(
    source: &str,
    prefix: Option<(&SyntaxTree, usize)>,
) -> (Vec<Node>, Option<SyntaxTree>) {
    let input = Input::new(source, false);
    let fragment = prefix.map(|(tree, end)| input.fragment(tree, end));
    let (mut arena, strict) = match PARSER.parse_incremental(&input.text, fragment) {
        Ok(tree) => (flatten(&tree), Some(tree)),
        Err(ParseError::Ambiguous { .. }) => (
            PARSER
                .parse_glr(&input.text)
                .map_or_else(|_| recovery::parse(&input.text), |tree| flatten(&tree)),
            None,
        ),
        Err(_) => (recovery::parse(&input.text), None),
    };
    for node in &mut arena.0 {
        node.range = input.byte(node.range.start)..input.byte(node.range.end);
    }
    arena.0[0].range.end = source.len();
    (arena.0, strict)
}

fn flatten(tree: &SyntaxTree) -> Arena {
    let mut arena = Arena::default();
    arena.copy(tree.root(), None, |range| Some((range, false)));
    arena
}

pub(super) fn parse_tree(source: &str) -> Result<SyntaxTree, ParseError> {
    PARSER.parse(source).or_else(|error| match error {
        ParseError::Ambiguous { .. } => PARSER.parse_glr(source),
        _ => Err(error),
    })
}

#[derive(Default)]
pub(super) struct Arena(pub Vec<Node>);

impl Arena {
    pub fn add(
        &mut self,
        kind: LiquidNodeKind,
        range: Range<usize>,
        parent: Option<usize>,
    ) -> usize {
        let id = self.0.len();
        if let Some(parent) = parent {
            self.0[parent].children.push(id);
        }
        self.0.push(Node {
            kind,
            range,
            parent,
            children: Vec::new(),
        });
        id
    }

    pub fn leaf(&mut self, kind: LiquidNodeKind, range: Range<usize>, parent: usize) -> usize {
        self.add(kind, range, Some(parent))
    }

    pub fn error(&mut self, at: usize, parent: usize) {
        self.leaf(Error, at..at, parent);
    }

    pub fn copy(
        &mut self,
        syntax: SyntaxNode<'_>,
        parent: Option<usize>,
        project: impl Fn(Range<usize>) -> Option<(Range<usize>, bool)>,
    ) -> Option<usize> {
        let mut stack = vec![(syntax, parent)];
        let mut result = None;
        while let Some((node, parent)) = stack.pop() {
            let Some((range, missing)) = project(node.range()) else {
                continue;
            };
            let kind = if missing {
                Error
            } else {
                LiquidNodeKind::from_name(node.name())
            };
            let id = self.add(kind, range, parent);
            result.get_or_insert(id);
            if !missing {
                stack.extend(node.children().rev().map(|child| (child, Some(id))));
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn bundled_tables_accept_complex_liquid() {
        for source in [
            "<p>{{ product.title | default: 'É', fallback: true }}</p>",
            "{% if x and y %}ok{% else %}bye{% endif %}",
            "{% comment %}{% if x %}{% endcomment %}{% raw %}{{x}}{% endraw %}",
            "{% liquid\n  for item in collection\n    echo item.name\n  endfor\n  # comment\n%}",
            "{% for item in value reversed limit: 2 %}{{ item['name'].size }}{% endfor %}",
            "{% # inline %}",
            "{% render 'item' with entry as object, wide: false %}",
        ] {
            assert!(
                super::parse_tree(source).is_ok(),
                "{source:?}: {:?}",
                super::parse_tree(source)
            );
        }
    }
}
