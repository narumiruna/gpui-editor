// Port of @lezer/python 1.1.20 src/highlight.js. See docs/licenses/codemirror-lang-python.txt.
use std::{ops::Range, sync::LazyLock};

use super::{
    super::{HighlightSpan, HighlightTag, TagModifier},
    PythonNode,
};

macro_rules! styles {
    ($($style:ident => ($name:literal $(, $modifier:ident)*),)*) => {
        #[derive(Clone, Copy, Eq, PartialEq)]
        pub(super) enum Style { $($style,)* }

        static TAGS: LazyLock<Box<[HighlightTag]>> = LazyLock::new(|| Box::new([
            $(HighlightTag::new($name)$(.with_modifier(TagModifier::$modifier))*,)*
        ]));
    };
}

styles! {
    Modifier => ("modifier"),
    Control => ("controlKeyword"),
    OperatorKeyword => ("operatorKeyword"),
    Definition => ("definitionKeyword"),
    Module => ("moduleKeyword"),
    Keyword => ("keyword"),
    Bool => ("bool"),
    Null => ("null"),
    Variable => ("variableName"),
    Call => ("variableName", Function),
    Function => ("variableName", Definition, Function),
    Class => ("className", Definition),
    Property => ("propertyName"),
    Method => ("propertyName", Function),
    Comment => ("lineComment"),
    Number => ("number"),
    String => ("string"),
    Format => ("string", Special),
    Escape => ("escape"),
    Update => ("updateOperator"),
    Arith => ("arithmeticOperator"),
    Bit => ("bitwiseOperator"),
    Compare => ("compareOperator"),
    Assign => ("definitionOperator"),
    Punct => ("punctuation"),
    Meta => ("meta"),
    Paren => ("paren"),
    Square => ("squareBracket"),
    Brace => ("brace"),
    Dot => ("derefOperator"),
    Separator => ("separator"),
}

pub(super) fn style(name: &str, parent: Option<&str>, grandparent: Option<&str>) -> Option<Style> {
    use Style::*;
    Some(match name {
        "async" | "*" | "**" | "FormatConversion" | "FormatSpec" => Modifier,
        "for" | "while" | "if" | "elif" | "else" | "try" | "except" | "finally" | "return"
        | "raise" | "break" | "continue" | "pass" | "assert" | "await" | "yield" | "match"
        | "case" | "with" => Control,
        "in" | "not" | "and" | "or" | "is" | "del" => OperatorKeyword,
        "from" | "def" | "class" | "global" | "nonlocal" | "lambda" => Definition,
        "import" => Module,
        "as" | "print" => Keyword,
        "Boolean" => Bool,
        "None" => Null,
        "VariableName" => match parent {
            Some("CallExpression") => Call,
            Some("FunctionDefinition") => Function,
            Some("ClassDefinition") => Class,
            _ => Variable,
        },
        "PropertyName"
            if parent == Some("MemberExpression") && grandparent == Some("CallExpression") =>
        {
            Method
        }
        "PropertyName" => Property,
        "Comment" => Comment,
        "Number" => Number,
        "String" => String,
        "FormatString" => Format,
        "Escape" => Escape,
        "UpdateOp" => Update,
        "ArithOp" => Arith,
        "BitOp" => Bit,
        "CompareOp" => Compare,
        "AssignOp" => Assign,
        "Ellipsis" => Punct,
        "At" => Meta,
        "(" | ")" => Paren,
        "[" | "]" => Square,
        "{" | "}" => Brace,
        "." => Dot,
        "," | ";" => Separator,
        _ => return None,
    })
}

pub(super) fn push(
    spans: &mut Vec<HighlightSpan<'static>>,
    range: Range<usize>,
    style: Option<Style>,
) {
    let Some(style) = style.filter(|_| !range.is_empty()) else {
        return;
    };
    let tags = std::slice::from_ref(&TAGS[style as usize]);
    spans.push(HighlightSpan { range, tags });
}

pub(super) fn collect(root: PythonNode<'_>) -> Vec<HighlightSpan<'static>> {
    enum Visit<'a> {
        Enter(PythonNode<'a>),
        Exit(usize, Option<Style>),
    }
    let mut spans = Vec::new();
    let mut visits = vec![Visit::Enter(root)];
    let (mut at, mut current) = (0, None);
    while let Some(visit) = visits.pop() {
        let (pos, next) = match visit {
            Visit::Enter(node) => {
                let range = node.range();
                if range.is_empty() {
                    continue;
                }
                let parent = node.parent();
                let next = style(
                    node.name(),
                    parent.map(PythonNode::name),
                    parent.and_then(PythonNode::parent).map(PythonNode::name),
                );
                visits.push(Visit::Exit(range.end, current));
                // The `ArithOp!` rule is opaque.
                if next != Some(Style::Arith) {
                    visits.extend(node.children().rev().map(Visit::Enter));
                }
                (range.start, next)
            }
            Visit::Exit(end, parent) => (end, parent),
        };
        if current != next {
            push(&mut spans, at..pos.max(at), current);
            at = at.max(pos);
            current = next;
        }
    }
    spans
}
