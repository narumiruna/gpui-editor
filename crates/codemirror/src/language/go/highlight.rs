use std::sync::LazyLock;

use super::{
    super::{HighlightSpan, HighlightTag, TagModifier},
    GoNode,
    GoNodeKind::*,
};

// Keep the tag identities from @lezer/go's styleTags rules, not a theme's
// CSS classes. Context paths (such as FunctionDecl/DefName) are direct.
fn tags(node: GoNode<'_>) -> Option<&'static [HighlightTag]> {
    use TagModifier::{Definition, Function};
    static TAGS: LazyLock<[HighlightTag; 33]> = LazyLock::new(|| {
        [
            HighlightTag::new("definitionKeyword"),
            HighlightTag::new("moduleKeyword"),
            HighlightTag::new("controlKeyword"),
            HighlightTag::new("keyword"),
            HighlightTag::new("bool"),
            HighlightTag::new("string"),
            HighlightTag::new("character"),
            HighlightTag::new("number"),
            HighlightTag::new("null"),
            HighlightTag::new("variableName"),
            HighlightTag::new("variableName").with_modifier(Definition),
            HighlightTag::new("typeName"),
            HighlightTag::new("labelName"),
            HighlightTag::new("propertyName"),
            HighlightTag::new("variableName")
                .with_modifier(Definition)
                .with_modifier(Function),
            HighlightTag::new("typeName").with_modifier(Definition),
            HighlightTag::new("variableName").with_modifier(Function),
            HighlightTag::new("lineComment"),
            HighlightTag::new("blockComment"),
            HighlightTag::new("logicOperator"),
            HighlightTag::new("arithmeticOperator"),
            HighlightTag::new("bitwiseOperator"),
            HighlightTag::new("derefOperator"),
            HighlightTag::new("updateOperator"),
            HighlightTag::new("compareOperator"),
            HighlightTag::new("definitionOperator"),
            HighlightTag::new("operator"),
            HighlightTag::new("modifier"),
            HighlightTag::new("separator"),
            HighlightTag::new("punctuation"),
            HighlightTag::new("paren"),
            HighlightTag::new("squareBracket"),
            HighlightTag::new("brace"),
        ]
    });
    let parent = node.parent().map(GoNode::kind);
    let index = match node.kind() {
        Func | Interface | Struct | Chan | Map | Const | Type | Var => 0,
        Import | Package => 1,
        Switch | For | Go | Select | Return | Break | Continue | Goto | Fallthrough
        | CaseKeyword | If | Else | Defer => 2,
        Range => 3,
        Bool => 4,
        String => 5,
        Rune => 6,
        Number => 7,
        Nil => 8,
        VariableName if parent == Some(CallExpr) => 16,
        VariableName => 9,
        DefName if parent == Some(FunctionDecl) => 14,
        DefName if parent == Some(TypeSpec) => 15,
        DefName => 10,
        TypeName => 11,
        LabelName => 12,
        FieldName => 13,
        LineComment => 17,
        BlockComment => 18,
        LogicOp => 19,
        ArithOp => 20,
        BitOp => 21,
        DerefOp | Dot => 22,
        UpdateOp | IncDecOp => 23,
        CompareOp => 24,
        Equal | Define => 25,
        Arrow => 26,
        Tilde | Star => 27,
        Semicolon | Comma => 28,
        Ellipsis | Colon => 29,
        OpenParen | CloseParen => 30,
        OpenBracket | CloseBracket => 31,
        OpenBrace | CloseBrace => 32,
        _ => return None,
    };
    Some(std::slice::from_ref(&TAGS[index]))
}

pub(super) fn collect(root: GoNode<'_>) -> Vec<HighlightSpan<'static>> {
    type Tags = Option<&'static [HighlightTag]>;
    enum Event<'a> {
        Enter(GoNode<'a>, Tags),
        Restore(usize, Tags),
    }

    let mut spans: Vec<HighlightSpan<'static>> = Vec::new();
    let mut active: Tags = None;
    let mut from = 0;
    let mut switch = |at: usize, next: Tags| {
        if next != active {
            if let Some(tags) = active
                && from < at
            {
                spans.push(HighlightSpan {
                    range: from..at,
                    tags,
                });
            }
            from = from.max(at);
            active = next;
        }
    };

    // Go's rules are not opaque. A recovered child can replace a parent's
    // style, and the parent style resumes after that child.
    let end = root.range().end;
    let mut pending = vec![Event::Enter(root, None)];
    while let Some(event) = pending.pop() {
        match event {
            Event::Enter(node, parent) => {
                let range = node.range();
                if range.start >= end || range.end == 0 {
                    continue;
                }
                let next = tags(node);
                switch(range.start, next);
                let children = node.children();
                if children.len() == 0 {
                    switch(range.end, parent);
                } else {
                    pending.push(Event::Restore(range.end, parent));
                    pending.extend(children.rev().map(|child| Event::Enter(child, next)));
                }
            }
            Event::Restore(at, parent) => switch(at, parent),
        }
    }
    spans
}
