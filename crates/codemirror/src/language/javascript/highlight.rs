// Style rules from @lezer/javascript 1.5.7 src/highlight.js.
use std::sync::LazyLock;

use super::{
    super::{HighlightSpan, HighlightTag, TagModifier},
    JavascriptNode,
};

pub(super) fn collect(root: JavascriptNode<'_>) -> Vec<HighlightSpan<'static>> {
    enum Work<'a> {
        Visit(JavascriptNode<'a>),
        Style(usize, Option<&'static [HighlightTag]>),
    }
    let mut spans = Vec::new();
    let mut start = 0;
    let mut current = None;
    let mut pending = vec![Work::Style(root.range().end, None), Work::Visit(root)];
    while let Some(work) = pending.pop() {
        match work {
            Work::Visit(node) => {
                let tags = tags(node);
                for child in node.children().rev() {
                    if !child.range().is_empty() {
                        // Restore the parent's style even at a zero-width gap.
                        pending.push(Work::Style(child.range().end, tags));
                        pending.push(Work::Visit(child));
                    }
                }
                pending.push(Work::Style(node.range().start, tags));
            }
            Work::Style(end, next) if next != current => {
                if let Some(tags) = current
                    && start < end
                {
                    spans.push(HighlightSpan {
                        range: start..end,
                        tags,
                    });
                }
                start = start.max(end);
                current = next;
            }
            Work::Style(..) => {}
        }
    }
    spans
}

fn tags(node: JavascriptNode<'_>) -> Option<&'static [HighlightTag]> {
    macro_rules! tag {
        ($name:literal $(, $mod:ident)* $(,)?) => {{
            static TAG: LazyLock<HighlightTag> = LazyLock::new(|| {
                HighlightTag::new($name)$(.with_modifier(TagModifier::$mod))*
            });
            std::slice::from_ref(&*TAG)
        }};
    }
    let parent = node.parent();
    let parent_name = parent.map(|node| node.kind().name());
    Some(match node.kind().name() {
        "get" | "set" | "async" | "static" | "Star" | "abstract" | "global" | "Privacy"
        | "readonly" | "override" => tag!("modifier"),
        "for" | "while" | "do" | "if" | "else" | "switch" | "try" | "catch" | "finally"
        | "return" | "throw" | "break" | "continue" | "default" | "case" | "defer" => {
            tag!("controlKeyword")
        }
        "in" | "of" | "await" | "yield" | "void" | "typeof" | "delete" | "instanceof" | "as"
        | "satisfies" | "is" | "keyof" | "unique" | "infer" | "asserts" => tag!("operatorKeyword"),
        "let" | "var" | "const" | "using" | "function" | "class" | "extends" | "type" | "enum"
        | "interface" | "implements" | "namespace" | "module" | "declare" => {
            tag!("definitionKeyword")
        }
        "import" | "export" | "from" => tag!("moduleKeyword"),
        "with" | "debugger" | "new" => tag!("keyword"),
        "TemplateString" => tag!("string", Special),
        "super" => tag!("atom"),
        "BooleanLiteral" => tag!("bool"),
        "this" => tag!("self"),
        "null" => tag!("null"),
        "VariableName"
            if matches!(
                parent_name,
                Some("CallExpression" | "TaggedTemplateExpression")
            ) =>
        {
            tag!("variableName", Function)
        }
        "VariableName" if parent_name == Some("NewExpression") => tag!("className"),
        "VariableName" => tag!("variableName"),
        "VariableDefinition" if parent_name == Some("FunctionDeclaration") => {
            tag!("variableName", Function, Definition)
        }
        "VariableDefinition" if parent_name == Some("ClassDeclaration") => {
            tag!("className", Definition)
        }
        "VariableDefinition" => tag!("variableName", Definition),
        "Label" => tag!("labelName"),
        "PropertyName"
            if parent_name == Some("MemberExpression")
                && parent
                    .and_then(JavascriptNode::parent)
                    .is_some_and(|node| node.kind().name() == "CallExpression") =>
        {
            tag!("propertyName", Function)
        }
        "PropertyName" => tag!("propertyName"),
        "PrivatePropertyName" => tag!("propertyName", Special),
        "PropertyDefinition" => tag!("propertyName", Definition),
        "PrivatePropertyDefinition" => tag!("propertyName", Special, Definition),
        "UpdateOp" => tag!("updateOperator"),
        "LineComment" | "Hashbang" => tag!("lineComment"),
        "BlockComment" => tag!("blockComment"),
        "Number" => tag!("number"),
        "String" => tag!("string"),
        "Escape" => tag!("escape"),
        "ArithOp" => tag!("arithmeticOperator"),
        "LogicOp" => tag!("logicOperator"),
        "BitOp" => tag!("bitwiseOperator"),
        "CompareOp" => tag!("compareOperator"),
        "RegExp" => tag!("regexp"),
        "Equals" => tag!("definitionOperator"),
        "Arrow" => tag!("punctuation", Function),
        ":" | "Spread" => tag!("punctuation"),
        "(" | ")" => tag!("paren"),
        "[" | "]" => tag!("squareBracket"),
        "{" | "}" => tag!("brace"),
        "InterpolationStart" | "InterpolationEnd" => tag!("brace", Special),
        "." => tag!("derefOperator"),
        "," | ";" => tag!("separator"),
        "@" => tag!("meta"),
        "TypeName" => tag!("typeName"),
        "TypeDefinition" => tag!("typeName", Definition),
        "JSXAttributeValue" => tag!("attributeValue"),
        "JSXText" => tag!("content"),
        "JSXStartTag" | "JSXStartCloseTag" | "JSXSelfCloseEndTag" | "JSXEndTag" => {
            tag!("angleBracket")
        }
        "JSXIdentifier" | "JSXNameSpacedName" if parent_name == Some("JSXAttribute") => {
            tag!("attributeName")
        }
        "JSXIdentifier" if parent_name == Some("JSXBuiltin") => tag!("tagName", Standard),
        "JSXIdentifier" | "JSXNameSpacedName" => tag!("tagName"),
        _ => return None,
    })
}
