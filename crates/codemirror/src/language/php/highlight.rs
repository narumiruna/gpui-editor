use std::{ops::Range, sync::LazyLock};

use super::{
    super::{HighlightSpan, HighlightTag, TagModifier},
    PhpNode,
};

const TYPE: usize = 1;
const NAMESPACE: usize = 2;

pub(super) fn collect(root: PhpNode<'_>) -> Vec<HighlightSpan<'static>> {
    let mut spans = Vec::new();
    // A node's own style covers the gaps between children. Only the `/...`
    // rules carry their tags into children.
    enum Event<'a> {
        Visit(PhpNode<'a>, usize),
        Paint(usize, &'static [HighlightTag]),
    }
    let mut stack = vec![Event::Visit(root, 0)];
    let mut cursor = 0;
    while let Some(event) = stack.pop() {
        let (node, inherited) = match event {
            Event::Visit(node, inherited) => (node, inherited),
            Event::Paint(end, tags) => {
                if cursor < end {
                    push(&mut spans, cursor..end, tags);
                    cursor = end;
                }
                continue;
            }
        };
        let range = node.range();
        if range.is_empty() {
            continue;
        }
        let tags = tags(inherited, style(node));
        let child_tags = match node.name() {
            "NamedType" => inherited | TYPE,
            "NamespaceName" => inherited | NAMESPACE,
            _ => inherited,
        };
        stack.push(Event::Paint(range.end, tags));
        for child in node.children().rev() {
            stack.push(Event::Visit(child, child_tags));
            stack.push(Event::Paint(child.range().start, tags));
        }
    }
    spans
}

fn push(
    spans: &mut Vec<HighlightSpan<'static>>,
    range: Range<usize>,
    tags: &'static [HighlightTag],
) {
    if range.is_empty() || tags.is_empty() {
        return;
    }
    if let Some(last) = spans.last_mut()
        && last.range.end == range.start
        && last.tags == tags
    {
        last.range.end = range.end;
    } else {
        spans.push(HighlightSpan { range, tags });
    }
}

use TagModifier::{Definition, Function, Special};

const STYLES: &[(&str, &[TagModifier])] = &[
    ("", &[]),
    ("modifier", &[]),
    ("controlKeyword", &[]),
    ("operatorKeyword", &[]),
    ("definitionKeyword", &[]),
    ("moduleKeyword", &[]),
    ("keyword", &[]),
    ("null", &[]),
    ("bool", &[]),
    ("variableName", &[]),
    ("name", &[]),
    ("variableName", &[Function]),
    ("labelName", &[]),
    ("propertyName", &[]),
    ("propertyName", &[Special]),
    ("propertyName", &[Function]),
    ("variableName", &[Definition, Function]),
    ("className", &[Definition]),
    ("updateOperator", &[]),
    ("arithmeticOperator", &[]),
    ("logicOperator", &[]),
    ("bitwiseOperator", &[]),
    ("compareOperator", &[]),
    ("controlOperator", &[]),
    ("definitionOperator", &[]),
    ("operator", &[]),
    ("lineComment", &[]),
    ("blockComment", &[]),
    ("integer", &[]),
    ("float", &[]),
    ("string", &[]),
    ("string", &[Special]),
    ("punctuation", &[]),
    ("paren", &[]),
    ("squareBracket", &[]),
    ("brace", &[]),
    ("derefOperator", &[]),
    ("separator", &[]),
    ("processingInstruction", &[]),
    ("namespace", &[]),
    ("typeName", &[]),
];

fn tags(inherited: usize, own: usize) -> &'static [HighlightTag] {
    static TAGS: LazyLock<Vec<Vec<HighlightTag>>> = LazyLock::new(|| {
        (0..4)
            .flat_map(|inherited| {
                STYLES.iter().map(move |&(name, modifiers)| {
                    let mut tags = Vec::new();
                    if inherited & TYPE != 0 {
                        tags.push(HighlightTag::new("typeName"));
                    }
                    if inherited & NAMESPACE != 0 {
                        tags.push(HighlightTag::new("namespace"));
                    }
                    if !name.is_empty() {
                        tags.push(
                            modifiers
                                .iter()
                                .fold(HighlightTag::new(name), |tag, &modifier| {
                                    tag.with_modifier(modifier)
                                }),
                        );
                    }
                    tags
                })
            })
            .collect()
    });
    &TAGS[inherited * STYLES.len() + own]
}

fn style(node: PhpNode<'_>) -> usize {
    let parent = node.parent();
    let parent_name = parent.map(PhpNode::name);
    let grand = parent.and_then(PhpNode::parent);
    let grand_name = grand.map(PhpNode::name);
    match node.name() {
        "Visibility" | "abstract" | "final" | "static" => 1,
        "for" | "foreach" | "while" | "do" | "if" | "else" | "elseif" | "switch" | "try"
        | "catch" | "finally" | "return" | "throw" | "break" | "continue" | "default" | "case"
        | "endif" | "endfor" | "endforeach" | "endswitch" | "endwhile" | "declare"
        | "enddeclare" | "goto" | "match" => 2,
        "and" | "or" | "xor" | "yield" | "unset" | "clone" | "instanceof" | "insteadof" => 3,
        "function" | "fn" | "class" | "trait" | "implements" | "extends" | "const" | "enum"
        | "global" | "interface" | "use" | "var" => 4,
        "include" | "include_once" | "require" | "require_once" | "namespace" => 5,
        "new" | "from" | "echo" | "print" | "array" | "list" | "as" => 6,
        "null" => 7,
        "Boolean" => 8,
        "VariableName" => match (parent_name, grand_name) {
            (Some("MemberExpression"), _) | (Some("ClassMemberName"), Some("ScopedExpression")) => {
                14
            }
            _ => 9,
        },
        "Name" => match (parent_name, grand_name) {
            (Some("CallExpression"), _) => 11,
            (Some("LabelStatement"), _) => 12,
            (Some("MemberExpression"), Some("CallExpression")) => 15,
            (Some("ClassMemberName"), Some("ScopedExpression")) => {
                if grand
                    .and_then(PhpNode::parent)
                    .is_some_and(|node| node.name() == "CallExpression")
                {
                    15
                } else {
                    13
                }
            }
            (Some("MemberExpression"), _) => 13,
            (Some("MethodDeclaration" | "FunctionDefinition"), _) => 16,
            (Some("ClassDeclaration"), _) => 17,
            _ => 10,
        },
        "UpdateOp" => 18,
        "ArithOp" => 19,
        "LogicOp" => 20,
        "&" if parent_name == Some("IntersectionType") => 20,
        "BitOp" => 21,
        "CompareOp" => 22,
        "ControlOp" => 23,
        "AssignOp" => 24,
        "$" | "ConcatOp" => 25,
        "LineComment" => 26,
        "BlockComment" => 27,
        "Integer" => 28,
        "Float" => 29,
        "String" => 30,
        "ShellExpression" => 31,
        "=>" | "->" => 32,
        "(" | ")" => 33,
        "#[" | "[" | "]" => 34,
        "${" | "{" | "}" => 35,
        "?->" => 36,
        "," | ";" | "::" | ":" | "\\" => 37,
        "PhpOpen" | "PhpClose" => 38,
        "NamespaceName" => 39,
        "NamedType" => 40,
        _ => 0,
    }
}
