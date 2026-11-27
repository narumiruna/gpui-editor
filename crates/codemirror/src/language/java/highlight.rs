// Style rules from @lezer/java 1.2.1 src/highlight.js.
use std::sync::LazyLock;

use super::{
    super::{HighlightSpan, HighlightTag, TagModifier},
    JavaDocument, JavaNode,
};

pub(super) fn collect(doc: &JavaDocument) -> Vec<HighlightSpan<'static>> {
    // Most styled nodes are leaves. A recovered styled node can contain
    // error markers and deleted tokens with styles of their own.
    let mut spans = Vec::new();
    let mut at = 0;
    let mut active: &[_] = &[];
    let mut pending = vec![(doc.root(), None)];
    while let Some((node, restore)) = pending.pop() {
        let (point, next) = if let Some(parent_tags) = restore {
            (node.data().range.end, parent_tags)
        } else {
            let next = tags(node).unwrap_or_default();
            pending.push((node, Some(active)));
            pending.extend(node.children().rev().map(|child| (child, None)));
            (node.data().range.start, next)
        };
        if active != next {
            if at < point && !active.is_empty() {
                spans.push(HighlightSpan {
                    range: at..point,
                    tags: active,
                });
            }
            at = at.max(point);
            active = next;
        }
    }
    spans
}

fn tags(node: JavaNode<'_>) -> Option<&'static [HighlightTag]> {
    macro_rules! tag {
        ($name:literal $(, $modifier:ident)?) => {{
            static TAG: LazyLock<HighlightTag> = LazyLock::new(|| {
                HighlightTag::new($name)$(.with_modifier(TagModifier::$modifier))?
            });
            std::slice::from_ref(&*TAG)
        }};
    }
    Some(match node.name() {
        "null" => tag!("null"),
        "instanceof" => tag!("operatorKeyword"),
        "this" => tag!("self"),
        "new" | "super" | "assert" | "open" | "to" | "with" | "void" => tag!("keyword"),
        "class" | "interface" | "extends" | "implements" | "enum" | "var" => {
            tag!("definitionKeyword")
        }
        "module" | "package" | "import" => tag!("moduleKeyword"),
        "switch" | "while" | "for" | "if" | "else" | "case" | "default" | "do" | "break"
        | "continue" | "return" | "try" | "catch" | "finally" | "throw" | "yield" => {
            tag!("controlKeyword")
        }
        "requires" | "exports" | "opens" | "uses" | "provides" | "public" | "private"
        | "protected" | "static" | "transitive" | "abstract" | "final" | "strictfp"
        | "synchronized" | "native" | "transient" | "volatile" | "throws" => tag!("modifier"),
        "IntegerLiteral" => tag!("integer"),
        "FloatingPointLiteral" => tag!("float"),
        "StringLiteral" | "TextBlock" => tag!("string"),
        "CharacterLiteral" => tag!("character"),
        "LineComment" => tag!("lineComment"),
        "BlockComment" => tag!("blockComment"),
        "BooleanLiteral" => tag!("bool"),
        "PrimitiveType" => tag!("typeName", Standard),
        "TypeName" => tag!("typeName"),
        "Identifier"
            if node
                .parent()
                .is_some_and(|parent| parent.name() == "MethodName") =>
        {
            tag!("variableName", Function)
        }
        "Identifier" => tag!("variableName"),
        "Definition" => tag!("variableName", Definition),
        "ArithOp" => tag!("arithmeticOperator"),
        "LogicOp" => tag!("logicOperator"),
        "BitOp" => tag!("bitwiseOperator"),
        "CompareOp" => tag!("compareOperator"),
        "AssignOp" => tag!("definitionOperator"),
        "UpdateOp" => tag!("updateOperator"),
        "Asterisk" => tag!("punctuation"),
        "Label" => tag!("labelName"),
        "(" | ")" => tag!("paren"),
        "[" | "]" => tag!("squareBracket"),
        "{" | "}" => tag!("brace"),
        "." => tag!("derefOperator"),
        "," | ";" => tag!("separator"),
        _ => return None,
    })
}
