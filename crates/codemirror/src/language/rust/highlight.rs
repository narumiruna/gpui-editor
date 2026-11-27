// Port of @lezer/rust 1.0.3 src/highlight.js.
// See docs/licenses/codemirror-lang-rust.txt.
use std::sync::LazyLock;

use super::{
    super::{HighlightSpan, HighlightTag, TagModifier},
    Node,
    RustNodeKind::*,
};

macro_rules! styles {
    ($($kind:ident => ($name:literal $(, $modifier:ident)*),)*) => {
        #[derive(Clone, Copy)]
        enum Style { $($kind,)* }

        static TAGS: LazyLock<Box<[HighlightTag]>> = LazyLock::new(|| Box::new([
            $(HighlightTag::new($name)$(.with_modifier(TagModifier::$modifier))*,)*
        ]));
    };
}

styles! {
    DefinitionKeyword => ("definitionKeyword"),
    ModuleKeyword => ("moduleKeyword"),
    Modifier => ("modifier"),
    ControlKeyword => ("controlKeyword"),
    OperatorKeyword => ("operatorKeyword"),
    Keyword => ("keyword"),
    SelfName => ("self"),
    String => ("string"),
    Char => ("character"),
    RawString => ("string", Special),
    Boolean => ("bool"),
    Variable => ("variableName"),
    CalledVariable => ("variableName", Function),
    BoundVariable => ("variableName", Definition),
    BoundFunction => ("variableName", Definition, Function),
    Label => ("labelName"),
    Property => ("propertyName"),
    CalledProperty => ("propertyName", Function),
    Lifetime => ("variableName", Special),
    Namespace => ("namespace"),
    Type => ("typeName"),
    Macro => ("macroName"),
    Update => ("updateOperator"),
    LineComment => ("lineComment"),
    BlockComment => ("blockComment"),
    Integer => ("integer"),
    Float => ("float"),
    Arith => ("arithmeticOperator"),
    Logic => ("logicOperator"),
    Bit => ("bitwiseOperator"),
    Compare => ("compareOperator"),
    Define => ("definitionOperator"),
    Punctuation => ("punctuation"),
    Paren => ("paren"),
    SquareBracket => ("squareBracket"),
    Brace => ("brace"),
    Deref => ("derefOperator"),
    Operator => ("operator"),
    Separator => ("separator"),
    Meta => ("meta"),
}

static META_TAGS: LazyLock<Box<[[HighlightTag; 2]]>> = LazyLock::new(|| {
    TAGS.iter()
        .map(|tag| [TAGS[Style::Meta as usize].clone(), tag.clone()])
        .collect()
});

fn style(nodes: &[Node], id: usize, recovered: bool) -> Option<Style> {
    let node = &nodes[id];
    let parent = node.parent.map(|p| &nodes[p]);
    let grandparent = parent.and_then(|p| p.parent).map(|p| nodes[p].kind);
    let parent_kind = parent.map(|p| p.kind);
    let macro_name = parent_kind == Some(MacroInvocation)
        || (parent_kind == Some(ScopedIdentifier) && grandparent == Some(MacroInvocation));
    let previous = || {
        let siblings = &parent?.children;
        let index = siblings.binary_search(&id).ok()?;
        Some(nodes[siblings[index.checked_sub(1)?]].kind)
    };
    Some(match node.kind {
        ConstKeyword | MacroRulesKeyword | StructKeyword | UnionKeyword | EnumKeyword
        | TypeKeyword | FnKeyword | ImplKeyword | TraitKeyword | LetKeyword | StaticKeyword => {
            Style::DefinitionKeyword
        }
        ModKeyword | UseKeyword | CrateKeyword => Style::ModuleKeyword,
        PubKeyword | UnsafeKeyword | AsyncKeyword | MutKeyword | ExternKeyword | DefaultKeyword
        | MoveKeyword => Style::Modifier,
        ForKeyword | IfKeyword | ElseKeyword | LoopKeyword | WhileKeyword | MatchKeyword
        | ContinueKeyword | BreakKeyword | ReturnKeyword | AwaitKeyword => Style::ControlKeyword,
        AsKeyword | InKeyword | RefKeyword => Style::OperatorKeyword,
        WhereKeyword | Underscore | SuperKeyword | DynKeyword => Style::Keyword,
        SelfKeyword => Style::SelfName,
        String => Style::String,
        Char => Style::Char,
        RawString => Style::RawString,
        Boolean => Style::Boolean,
        Identifier | TypeIdentifier if macro_name => Style::Macro,
        Identifier if parent_kind == Some(CallExpression) => Style::CalledVariable,
        Identifier => Style::Variable,
        BoundIdentifier
            if parent_kind == Some(FunctionItem)
                || (recovered && previous() == Some(FnKeyword)) =>
        {
            Style::BoundFunction
        }
        BoundIdentifier => Style::BoundVariable,
        LoopLabel => Style::Label,
        FieldIdentifier
            if parent_kind == Some(FieldExpression) && grandparent == Some(CallExpression) =>
        {
            Style::CalledProperty
        }
        FieldIdentifier => Style::Property,
        Lifetime => Style::Lifetime,
        ScopeIdentifier => Style::Namespace,
        TypeIdentifier => Style::Type,
        Bang => Style::Macro,
        UpdateOp => Style::Update,
        LineComment | Shebang => Style::LineComment,
        BlockComment => Style::BlockComment,
        Integer => Style::Integer,
        Float => Style::Float,
        ArithOp => Style::Arith,
        LogicOp => Style::Logic,
        BitOp => Style::Bit,
        CompareOp => Style::Compare,
        Equal => Style::Define,
        DoubleDot | TripleDot | Arrow | FatArrow => Style::Punctuation,
        OpenParen | CloseParen => Style::Paren,
        OpenBracket | CloseBracket => Style::SquareBracket,
        OpenBrace | CloseBrace => Style::Brace,
        DerefOp => Style::Deref,
        Ampersand => Style::Operator,
        Comma | Semicolon | DoubleColon => Style::Separator,
        _ => return None,
    })
}

fn tags(style: Option<Style>, meta: bool) -> &'static [HighlightTag] {
    match (style, meta) {
        (Some(style), true) => &META_TAGS[style as usize],
        (Some(style), false) => std::slice::from_ref(&TAGS[style as usize]),
        (None, true) => std::slice::from_ref(&TAGS[Style::Meta as usize]),
        (None, false) => &[],
    }
}

pub(super) fn collect(nodes: &[Node], recovered: bool) -> Vec<HighlightSpan<'static>> {
    let mut spans: Vec<HighlightSpan<'static>> = Vec::new();
    let mut transition = |at, next, run: &mut (usize, &'static [HighlightTag])| {
        let (start, current) = *run;
        if current != next {
            if start < at && !current.is_empty() {
                spans.push(HighlightSpan {
                    range: start..at,
                    tags: current,
                });
            }
            *run = (at, next);
        }
    };
    let mut ancestors: Vec<(usize, usize, &'static [HighlightTag], bool)> = Vec::new();
    let mut run = (0, &[] as &[_]);
    let mut meta = false;
    for (id, node) in nodes
        .iter()
        .enumerate()
        .filter(|(_, node)| !node.range.is_empty())
    {
        while ancestors
            .last()
            .is_some_and(|&(parent, ..)| Some(parent) != node.parent)
        {
            let (_, end, restore, inherited) = ancestors.pop().unwrap();
            transition(end, restore, &mut run);
            meta = inherited;
        }
        ancestors.push((id, node.range.end, run.1, meta));
        meta |= node.kind == Attribute;
        transition(
            node.range.start,
            tags(style(nodes, id, recovered), meta),
            &mut run,
        );
    }
    while let Some((_, end, restore, _)) = ancestors.pop() {
        transition(end, restore, &mut run);
    }
    spans
}
