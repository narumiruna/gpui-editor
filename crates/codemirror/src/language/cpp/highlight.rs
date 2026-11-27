use std::sync::LazyLock;

use super::{
    super::{HighlightSpan, HighlightTag, TokenTable, highlight::Style},
    CppNode,
    CppNodeKind::*,
};

macro_rules! styles {
    ($($style:ident => $name:literal),* $(,)?) => {
        #[derive(Clone, Copy, Eq, PartialEq)]
        enum Tag { $($style,)* }

        impl Tag {
            fn tags(self) -> &'static [HighlightTag] {
                static TAGS: LazyLock<Vec<Vec<HighlightTag>>> = LazyLock::new(|| {
                    let custom = TokenTable::new();
                    [$($name,)*].into_iter().map(|name| {
                        Style::resolve(name, &custom).expect("valid upstream highlight tag").tags
                    }).collect()
                });
                &TAGS[self as usize]
            }
        }
    };
}

styles! {
    DefinitionKeyword => "definitionKeyword",
    Modifier => "modifier",
    ControlKeyword => "controlKeyword",
    OperatorKeyword => "operatorKeyword",
    Null => "null",
    SelfName => "self",
    Bool => "bool",
    StandardType => "typeName.standard",
    Type => "typeName",
    Property => "propertyName",
    FunctionProperty => "propertyName.function",
    Namespace => "namespace",
    Label => "labelName",
    Variable => "variableName",
    FunctionVariable => "variableName.function",
    FunctionDefinition => "variableName.definition.function",
    Operator => "operator",
    Arithmetic => "arithmeticOperator",
    Logic => "logicOperator",
    Bitwise => "bitwiseOperator",
    Compare => "compareOperator",
    Update => "updateOperator",
    LineComment => "lineComment",
    BlockComment => "blockComment",
    Number => "number",
    String => "string",
    SpecialString => "string.special",
    Character => "character",
    Escape => "escape",
    Literal => "literal",
    Meta => "meta",
    ProcessingInstruction => "processingInstruction",
    Macro => "name.special",
    Paren => "paren",
    SquareBracket => "squareBracket",
    Brace => "brace",
    AngleBracket => "angleBracket",
    Separator => "separator",
}

fn style(node: CppNode<'_>) -> Option<Tag> {
    let parent = || node.parent().map(CppNode::kind);
    let grandparent = || node.parent()?.parent().map(CppNode::kind);
    Some(match node.kind() {
        Typedef
        | StructKeyword
        | UnionKeyword
        | EnumKeyword
        | ClassKeyword
        | Typename
        | DecltypeKeyword
        | Auto
        | Template
        | Operator
        | Friend
        | NoexceptKeyword
        | Namespace
        | Using
        | Requires
        | Concept
        | Import
        | Export
        | Module
        | DoubleUnderscoreAttribute
        | DoubleUnderscoreDeclspec
        | DoubleUnderscoreBased => Tag::DefinitionKeyword,
        Extern | MsCallModifier | MsPointerModifier | Static | Register | ThreadLocal | Inline
        | Const | Volatile | Restrict | UnderscoreAtomic | Mutable | Constexpr | Constinit
        | Consteval | Virtual | Explicit | VirtualSpecifier | Access => Tag::Modifier,
        If | Else | Switch | For | While | Do | Case | Default | Return | Break | Continue
        | Goto | Throw | Try | Catch | CoReturn | CoYield | CoAwait => Tag::ControlKeyword,
        New | Sizeof | Delete | StaticAssert => Tag::OperatorKeyword,
        Null | Nullptr => Tag::Null,
        This => Tag::SelfName,
        True | False => Tag::Bool,
        TypeSize | PrimitiveType => Tag::StandardType,
        TypeIdentifier => Tag::Type,
        FieldIdentifier
            if parent() == Some(FieldExpression) && grandparent() == Some(CallExpression) =>
        {
            Tag::FunctionProperty
        }
        FieldIdentifier => Tag::Property,
        Identifier if parent() == Some(ModuleName) => Tag::Namespace,
        PartitionName | StatementIdentifier => Tag::Label,
        Identifier
            if parent() == Some(CallExpression)
                || (parent() == Some(ScopedIdentifier)
                    && grandparent() == Some(CallExpression)) =>
        {
            Tag::FunctionVariable
        }
        Identifier | DestructorName if parent() == Some(FunctionDeclarator) => {
            Tag::FunctionDefinition
        }
        Identifier if parent() == Some(UserDefinedLiteral) => Tag::Literal,
        Identifier | DestructorName => Tag::Variable,
        NamespaceIdentifier => Tag::Namespace,
        OperatorName => Tag::Operator,
        ArithOp => Tag::Arithmetic,
        LogicOp => Tag::Logic,
        BitOp => Tag::Bitwise,
        CompareOp => Tag::Compare,
        UpdateOp => Tag::Update,
        LineComment => Tag::LineComment,
        BlockComment => Tag::BlockComment,
        Number => Tag::Number,
        String => Tag::String,
        RawString | SystemLibString => Tag::SpecialString,
        CharLiteral => Tag::Character,
        EscapeSequence => Tag::Escape,
        PreprocArg => Tag::Meta,
        PreprocDirectiveName | HashInclude | HashIfdef | HashIfndef | HashIf | HashDefine
        | HashElse | HashEndif | HashElif => Tag::ProcessingInstruction,
        MacroName => Tag::Macro,
        OpenParen | CloseParen => Tag::Paren,
        OpenBracket | CloseBracket => Tag::SquareBracket,
        OpenBrace | CloseBrace => Tag::Brace,
        OpenAngle | CloseAngle => Tag::AngleBracket,
        Comma => Tag::Separator,
        _ => return None,
    })
}

pub(super) fn collect(root: CppNode<'_>) -> Vec<HighlightSpan<'static>> {
    enum Event<'a> {
        Enter(CppNode<'a>),
        Restore(usize, Option<Tag>),
    }
    let mut spans = Vec::new();
    let mut active = None::<Tag>;
    let mut start = 0;
    let mut pending = vec![Event::Enter(root)];
    while let Some(event) = pending.pop() {
        let (at, next) = match event {
            Event::Enter(node) => {
                let range = node.range();
                if range.is_empty() {
                    continue;
                }
                let next = style(node);
                for child in node.children().rev() {
                    if !child.range().is_empty() {
                        pending.push(Event::Restore(child.data().range.end, next));
                        pending.push(Event::Enter(child));
                    }
                }
                (range.start, next)
            }
            Event::Restore(at, next) => (at, next),
        };
        if active != next {
            if let Some(tag) = active
                && start < at
            {
                spans.push(HighlightSpan {
                    range: start..at,
                    tags: tag.tags(),
                });
            }
            start = at;
            active = next;
        }
    }
    spans
}
