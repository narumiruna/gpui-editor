use std::{ops::Range, sync::LazyLock};

use super::{
    super::{HighlightTag, TagModifier},
    HighlightSpan, JinjaNode,
};

pub(super) fn collect(root: JinjaNode<'_>) -> Vec<HighlightSpan<'static>> {
    let mut spans = Vec::new();
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        match node.name() {
            "Comment" => {
                let mut start = node.range().start;
                for child in node.children() {
                    let range = child.range();
                    push(&mut spans, start..range.start, tags(0));
                    push(
                        &mut spans,
                        range.clone(),
                        if matches!(child.name(), "{#" | "#}") {
                            &COMMENT_DELIMITER[..]
                        } else {
                            tags(0)
                        },
                    );
                    start = range.end;
                }
                push(&mut spans, start..node.range().end, tags(0));
            }
            name => {
                if let Some(index) = tag(name) {
                    push(&mut spans, node.range(), tags(index));
                } else {
                    stack.extend(node.children().rev());
                }
            }
        }
    }
    spans
}

fn push(
    spans: &mut Vec<HighlightSpan<'static>>,
    range: Range<usize>,
    tags: &'static [HighlightTag],
) {
    if !range.is_empty() {
        spans.push(HighlightSpan { range, tags });
    }
}

static COMMENT_DELIMITER: LazyLock<[HighlightTag; 2]> = LazyLock::new(|| {
    [
        HighlightTag::new("blockComment"),
        HighlightTag::new("brace"),
    ]
});

fn tags(index: usize) -> &'static [HighlightTag] {
    static TAGS: LazyLock<[HighlightTag; 24]> = LazyLock::new(|| {
        use TagModifier::*;
        [
            ("blockComment", None),
            ("keyword", None),
            ("modifier", None),
            ("self", None),
            ("variableName", Some(Standard)),
            ("controlKeyword", None),
            ("definitionKeyword", None),
            ("variableName", None),
            ("variableName", Some(Definition)),
            ("propertyName", None),
            ("variableName", Some(Special)),
            ("arithmeticOperator", None),
            ("definitionOperator", None),
            ("logicOperator", None),
            ("compareOperator", None),
            ("operatorKeyword", None),
            ("operator", None),
            ("string", None),
            ("number", None),
            ("bool", None),
            ("brace", None),
            ("paren", None),
            ("derefOperator", None),
            ("punctuation", None),
        ]
        .map(|(name, modifier)| {
            let tag = HighlightTag::new(name);
            if let Some(modifier) = modifier {
                tag.with_modifier(modifier)
            } else {
                tag
            }
        })
    });
    std::slice::from_ref(&TAGS[index])
}

fn tag(name: &str) -> Option<usize> {
    Some(match name {
        "TagName" | "raw" | "endraw" | "filter" | "endfilter" | "as" | "trans" | "pluralize"
        | "endtrans" | "with" | "endwith" | "autoescape" | "endautoescape" => 1,
        "required" | "scoped" | "recursive" | "without" | "context" | "ignore" | "missing" => 2,
        "self" => 3,
        "loop" | "super" => 4,
        "if" | "elif" | "else" | "endif" | "for" | "endfor" | "call" | "endcall" => 5,
        "block" | "endblock" | "set" | "endset" | "macro" | "endmacro" | "import" | "from"
        | "include" => 6,
        "VariableName" => 7,
        "Definition" => 8,
        "PropertyName" => 9,
        "FilterName" => 10,
        "ArithOp" => 11,
        "AssignOp" => 12,
        "not" | "and" | "or" => 13,
        "CompareOp" => 14,
        "in" | "is" => 15,
        "FilterOp" | "ConcatOp" => 16,
        "StringLiteral" => 17,
        "NumberLiteral" => 18,
        "BooleanLiteral" => 19,
        "{%" | "%}" | "{#" | "#}" | "{{" | "}}" => 20,
        "(" | ")" => 21,
        "." => 22,
        ":" | "," => 23,
        _ => return None,
    })
}
