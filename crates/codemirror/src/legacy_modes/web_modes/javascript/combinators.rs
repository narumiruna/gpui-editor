// The upstream continuation stack, with explicit rules in place of JS closures.
use super::scanner::{Kind, Scan, Token, word};
use super::{JS_WS, Lex, Lexical, Scope, State, ascii_word, js_space};
use crate::stream_parser::StringStream;

#[derive(Clone, Copy, Debug)]
pub(super) enum Rule {
    PushLex(Lex, &'static str),
    PopLex,
    PushScope { block: bool },
    PopScope,
    Expect(Kind),
    Statement,
    MaybeCatchBinding,
    Expression(bool),
    ParenExpr,
    MaybeExpression,
    MaybeOperator(bool),
    Quasi(bool),
    ContinueQuasi(bool),
    ArrowBody(bool),
    MaybeTarget(bool),
    Target(bool),
    MaybeLabel,
    Property,
    ObjProp,
    GetterSetter,
    AfterProp,
    Separated(Item, char, Step),
    Block,
    MaybeType,
    MaybeTypeOrIn,
    MaybeRetType,
    IsKeyword,
    TypeExpr,
    MaybeReturnType,
    TypeProps,
    TypeProp,
    TypeArg,
    AfterType,
    MaybeTypeArgs,
    TypeParam,
    MaybeTypeDefault,
    VarDef,
    Pattern,
    PropPattern,
    ElementPattern,
    MaybeAssign,
    VarDefCont,
    MaybeElse,
    ForSpec,
    ForSpec1,
    ForSpec2,
    FunctionDef { body: bool },
    TypeName,
    FunArg,
    ClassExpression,
    ClassName,
    ClassNameAfter,
    ClassBody,
    ClassField,
    AfterExport,
    ExportField,
    AfterImport,
    ImportSpec,
    MaybeMoreImports,
    MaybeAs,
    MaybeFrom,
    ArrayLiteral,
    EnumDef,
    EnumMember,
}

#[derive(Clone, Copy, Debug)]
pub(super) enum Item {
    ObjProp,
    Expression,
    TypeExpr,
    TypeArg,
    TypeParam,
    ElementPattern,
    PropPattern,
    FunArg,
    ExportField,
    ImportSpec,
    EnumMember,
}

#[derive(Clone, Copy, Debug)]
pub(super) enum Step {
    First,
    Proceed,
    OptionalItem,
}

impl Item {
    fn rule(self) -> Rule {
        match self {
            Self::ObjProp => Rule::ObjProp,
            Self::Expression => Rule::Expression(true),
            Self::TypeExpr => Rule::TypeExpr,
            Self::TypeArg => Rule::TypeArg,
            Self::TypeParam => Rule::TypeParam,
            Self::ElementPattern => Rule::ElementPattern,
            Self::PropPattern => Rule::PropPattern,
            Self::FunArg => Rule::FunArg,
            Self::ExportField => Rule::ExportField,
            Self::ImportSpec => Rule::ImportSpec,
            Self::EnumMember => Rule::EnumMember,
        }
    }
}

fn separated(item: Item, end: char) -> Rule {
    Rule::Separated(item, end, Step::First)
}

fn bracket(ch: char) -> Rule {
    Rule::PushLex(Lex::Bracket(ch), "")
}

fn expect(ch: char) -> Rule {
    Rule::Expect(Kind::Punct(ch))
}

const EXPR: Rule = Rule::Expression(false);
const FUNCTION: Rule = Rule::FunctionDef { body: true };
const DECL: Rule = Rule::FunctionDef { body: false };
const BLOCK_SCOPE: Rule = Rule::PushScope { block: true };
const FUNCTION_SCOPE: Rule = Rule::PushScope { block: false };

impl State {
    pub(super) fn parse(
        &mut self,
        stream: &mut StringStream<'_>,
        token: Token<'_>,
    ) -> Option<&'static str> {
        self.lexical.last_mut().unwrap().align.get_or_insert(true);
        let mut parse = Parse {
            state: self,
            stream,
            token,
            marked: None,
        };
        let fallback = if parse.state.json() {
            EXPR
        } else {
            Rule::Statement
        };
        loop {
            let rule = parse.state.rules.pop().unwrap_or(fallback);
            if parse.apply(rule) {
                while parse
                    .state
                    .rules
                    .last()
                    .is_some_and(|rule| rule.is_lexical())
                {
                    let rule = parse.state.rules.pop().unwrap();
                    parse.apply(rule);
                }
                return parse
                    .marked
                    .or_else(|| {
                        (token.kind == Kind::Variable && parse.in_scope())
                            .then_some("variableName.local")
                    })
                    .or(token.style);
            }
        }
    }
}

impl Rule {
    fn is_lexical(self) -> bool {
        matches!(
            self,
            Self::PushLex(..) | Self::PopLex | Self::PushScope { .. } | Self::PopScope
        )
    }
}

struct Parse<'a, 'line, 'token> {
    state: &'a mut State,
    stream: &'a mut StringStream<'line>,
    token: Token<'token>,
    marked: Option<&'static str>,
}

impl Parse<'_, '_, '_> {
    fn push(&mut self, rules: impl IntoIterator<Item = Rule, IntoIter: DoubleEndedIterator>) {
        self.state.rules.extend(rules.into_iter().rev());
    }

    fn rest(&self) -> &str {
        &self.stream.line()[self.stream.pos()..]
    }

    fn in_scope(&self) -> bool {
        self.state
            .locals
            .iter()
            .chain(self.state.scopes.iter().flat_map(|scope| &scope.vars))
            .any(|name| name == self.token.value)
    }

    fn register(&mut self) {
        self.marked = Some("def");
        let state = &mut self.state;
        let Some(scope) = state.scopes.last() else {
            return;
        };
        let names = if scope.block && state.lexical().info == "var" {
            let Some(scope) = state.scopes.iter_mut().rev().find(|scope| !scope.block) else {
                return;
            };
            &mut scope.vars
        } else {
            &mut state.locals
        };
        if !names.iter().any(|name| name == self.token.value) {
            names.push(self.token.value.to_owned());
        }
    }

    // A successful rule consumes the token. A retry adds rules and presents
    // the same token to the next rule on the stack.
    fn apply(&mut self, rule: Rule) -> bool {
        use Kind::*;
        use Rule::*;
        let Token { kind, style, value } = self.token;
        let ts = self.state.ts();
        macro_rules! consume {
            ($($next:expr),* $(,)?) => {{ self.push([$($next),*]); return true; }};
        }
        macro_rules! retry {
            ($($next:expr),* $(,)?) => {{ self.push([$($next),*]); return false; }};
        }
        macro_rules! list {
            ($item:ident, $end:literal $(, $next:expr)?) => {
                consume![bracket($end), separated(Item::$item, $end), PopLex $(, $next)?]
            };
        }
        match rule {
            PushLex(kind, info) => {
                let state = &mut self.state;
                let mut indented = state.indented;
                if state.lexical().kind == Lex::Statement {
                    indented = state.lexical().indented;
                } else {
                    for outer in
                        state.lexical.iter().rev().take_while(|lex| {
                            lex.kind == Lex::Bracket(')') && lex.align == Some(true)
                        })
                    {
                        indented = outer.indented;
                    }
                }
                state.lexical.push(Lexical {
                    kind,
                    indented,
                    column: self.stream.column(),
                    align: None,
                    info,
                });
            }
            PopLex if self.state.lexical.len() > 1 => {
                let lex = self.state.lexical.pop().unwrap();
                if lex.kind == Lex::Bracket(')') {
                    self.state.indented = lex.indented;
                }
            }
            PushScope { block } => {
                let vars = std::mem::take(&mut self.state.locals);
                self.state.scopes.push(Scope { vars, block });
                if !block {
                    self.state.locals = vec!["this".into(), "arguments".into()];
                }
            }
            PopScope => {
                self.state.locals = self.state.scopes.pop().unwrap().vars;
            }
            Expect(wanted) => {
                if kind == wanted {
                    return true;
                }
                if wanted != Punct(';') && !matches!(kind, Punct('}' | ')' | ']')) {
                    consume![rule];
                }
            }
            Statement => match kind {
                Var => consume![
                    PushLex(
                        Lex::VarDef,
                        match value {
                            "const" => "const",
                            "let" => "let",
                            _ => "var",
                        }
                    ),
                    VarDef,
                    expect(';'),
                    PopLex
                ],
                KeywordA => consume![PushLex(Lex::Form, ""), ParenExpr, Statement, PopLex],
                KeywordB => consume![PushLex(Lex::Form, ""), Statement, PopLex],
                KeywordD if self.rest().chars().all(js_space) => return true,
                KeywordD => consume![
                    PushLex(Lex::Statement, ""),
                    MaybeExpression,
                    expect(';'),
                    PopLex
                ],
                Debugger => consume![expect(';')],
                Punct('{') => consume![bracket('}'), BLOCK_SCOPE, Block, PopLex, PopScope],
                Punct(';') => return true,
                If => {
                    if self.state.lexical().info == "else"
                        && matches!(self.state.rules.last(), Some(PopLex))
                    {
                        self.state.rules.pop();
                        self.apply(PopLex);
                    }
                    consume![
                        PushLex(Lex::Form, ""),
                        ParenExpr,
                        Statement,
                        PopLex,
                        MaybeElse
                    ];
                }
                Function => consume![FUNCTION],
                For => consume![
                    PushLex(Lex::Form, ""),
                    BLOCK_SCOPE,
                    ForSpec,
                    Statement,
                    PopScope,
                    PopLex
                ],
                Class | Variable if kind == Class || ts && value == "interface" => {
                    self.marked = Some("keyword");
                    consume![
                        PushLex(Lex::Form, if kind == Class { "class" } else { "interface" }),
                        ClassName,
                        PopLex
                    ];
                }
                Variable if ts && matches!(value, "declare" | "abstract") => {
                    self.marked = Some("keyword");
                    consume![Statement];
                }
                Variable
                    if ts
                        && matches!(value, "module" | "enum" | "type")
                        && self
                            .rest()
                            .trim_start_matches(js_space)
                            .starts_with(ascii_word) =>
                {
                    self.marked = Some("keyword");
                    match value {
                        "enum" => consume![EnumDef],
                        "type" => consume![TypeName, Expect(Operator), TypeExpr, expect(';')],
                        _ => consume![
                            PushLex(Lex::Form, ""),
                            Pattern,
                            expect('{'),
                            bracket('}'),
                            Block,
                            PopLex,
                            PopLex
                        ],
                    }
                }
                Variable if ts && value == "namespace" => {
                    self.marked = Some("keyword");
                    consume![PushLex(Lex::Form, ""), EXPR, Statement, PopLex];
                }
                Variable => consume![PushLex(Lex::Statement, ""), MaybeLabel],
                Switch => consume![
                    PushLex(Lex::Form, ""),
                    ParenExpr,
                    expect('{'),
                    PushLex(Lex::Bracket('}'), "switch"),
                    BLOCK_SCOPE,
                    Block,
                    PopLex,
                    PopLex,
                    PopScope
                ],
                Case => consume![EXPR, expect(':')],
                Default => consume![expect(':')],
                Catch => consume![
                    PushLex(Lex::Form, ""),
                    FUNCTION_SCOPE,
                    MaybeCatchBinding,
                    Statement,
                    PopLex,
                    PopScope
                ],
                Export => consume![PushLex(Lex::Statement, ""), AfterExport, PopLex],
                Import => consume![PushLex(Lex::Statement, ""), AfterImport, PopLex],
                Async => consume![Statement],
                _ if value == "@" => consume![EXPR, Statement],
                _ => retry![PushLex(Lex::Statement, ""), EXPR, expect(';'), PopLex],
            },
            MaybeCatchBinding if kind == Punct('(') => consume![FunArg, expect(')')],
            ParenExpr if kind == Punct('(') => {
                consume![bracket(')'), MaybeExpression, expect(')'), PopLex]
            }
            Expression(no_comma) => {
                if self.state.fat_arrow == Some(self.stream.start()) {
                    match kind {
                        Punct('(') => consume![
                            FUNCTION_SCOPE,
                            bracket(')'),
                            separated(Item::FunArg, ')'),
                            PopLex,
                            Expect(Arrow),
                            ArrowBody(no_comma),
                            PopScope
                        ],
                        Variable => retry![
                            FUNCTION_SCOPE,
                            Pattern,
                            Expect(Arrow),
                            ArrowBody(no_comma),
                            PopScope
                        ],
                        _ => {}
                    }
                }
                let op = MaybeOperator(no_comma);
                match kind {
                    Atom | Number | Variable | String | Regexp | This | Import | JsonLdKeyword => {
                        consume![op]
                    }
                    Function => consume![FUNCTION, op],
                    Class => {
                        self.marked = Some("keyword");
                        consume![PushLex(Lex::Form, ""), ClassExpression, PopLex];
                    }
                    KeywordC | Async | Operator | Spread => consume![rule],
                    Punct('(') => consume![bracket(')'), MaybeExpression, expect(')'), PopLex, op],
                    Punct('[') => consume![bracket(']'), ArrayLiteral, PopLex, op],
                    Punct('{') => list!(ObjProp, '}', op),
                    Kind::Quasi => retry![Rule::Quasi(false), op],
                    New => consume![MaybeTarget(no_comma)],
                    _ => return true,
                }
            }
            MaybeExpression if !matches!(kind, Punct(';' | '}' | ')' | ']' | ',')) => retry![EXPR],
            MaybeOperator(no_comma) => {
                let expr = Expression(no_comma);
                match kind {
                    Punct(',') if !no_comma => consume![MaybeExpression],
                    // The upstream no-comma combinator has no third argument
                    // when called by the parser, and uses the comma arrow body.
                    Arrow => consume![FUNCTION_SCOPE, ArrowBody(false), PopScope],
                    Operator
                        if value.contains("++") || value.contains("--") || ts && value == "!" =>
                    {
                        consume![rule]
                    }
                    Operator
                        if ts
                            && value == "<"
                            && re!(format!(r"^(?:[^<>]|<[^<>]*>)*>{JS_WS}*\("))
                                .is_match(self.rest()) =>
                    {
                        list!(TypeExpr, '>', rule)
                    }
                    Operator if value == "?" => consume![EXPR, expect(':'), expr],
                    Operator => consume![expr],
                    Kind::Quasi => retry![Rule::Quasi(false), rule],
                    Punct(';') => {}
                    Punct('(') => consume![
                        PushLex(Lex::Bracket(')'), "call"),
                        separated(Item::Expression, ')'),
                        PopLex,
                        rule
                    ],
                    Punct('.') => consume![Property, rule],
                    Punct('[') => {
                        consume![bracket(']'), MaybeExpression, expect(']'), PopLex, rule]
                    }
                    _ if ts && value == "as" => {
                        self.marked = Some("keyword");
                        consume![TypeExpr, rule];
                    }
                    Regexp => {
                        self.state.last = Operator;
                        self.marked = Some("operator");
                        self.stream.set_pos(self.stream.start() + 1).unwrap();
                        consume![expr];
                    }
                    _ => {}
                }
            }
            Rule::Quasi(typed) if kind == Kind::Quasi => {
                if value.ends_with("${") {
                    consume![
                        if typed { TypeExpr } else { MaybeExpression },
                        ContinueQuasi(typed)
                    ];
                }
                consume![rule];
            }
            ContinueQuasi(typed) if kind == Punct('}') => {
                self.marked = Some("string.special");
                self.state.scan = Scan::Quasi;
                consume![Rule::Quasi(typed)];
            }
            ArrowBody(no_comma) => {
                self.state.find_arrow(self.stream);
                retry![if kind == Punct('{') {
                    Statement
                } else {
                    Expression(no_comma)
                }];
            }
            MaybeTarget(no_comma) => {
                if kind == Punct('.') {
                    consume![Target(no_comma)];
                }
                if ts && kind == Variable {
                    consume![MaybeTypeArgs, MaybeOperator(no_comma)];
                }
                retry![Expression(no_comma)];
            }
            Target(no_comma) if value == "target" => {
                self.marked = Some("keyword");
                consume![MaybeOperator(no_comma)];
            }
            MaybeLabel if kind == Punct(':') => consume![PopLex, Statement],
            MaybeLabel => retry![MaybeOperator(false), expect(';'), PopLex],
            Property if kind == Variable => {
                self.marked = Some("property");
                return true;
            }
            ObjProp => {
                if kind == Async {
                    self.marked = Some("property");
                    consume![ObjProp];
                }
                if kind == Variable || style == Some("keyword") {
                    self.marked = Some("property");
                    if matches!(value, "get" | "set") {
                        consume![GetterSetter];
                    }
                    if ts
                        && self.state.fat_arrow == Some(self.stream.start())
                        && let Some(m) = re!(format!(r"^{JS_WS}*:{JS_WS}*")).find(self.rest())
                    {
                        self.state.fat_arrow = Some(self.stream.pos() + m.end());
                    }
                    consume![AfterProp];
                }
                match kind {
                    Number | String => {
                        self.marked = Some(if self.state.json_ld() {
                            "property"
                        } else if kind == Number {
                            "number property"
                        } else {
                            "string property"
                        });
                        consume![AfterProp];
                    }
                    JsonLdKeyword => consume![AfterProp],
                    Punct('[') => consume![EXPR, MaybeType, expect(']'), AfterProp],
                    Spread => consume![Expression(true), AfterProp],
                    Punct(':') => retry![AfterProp],
                    _ if value == "*" || ts && modifier(value) => {
                        self.marked = Some("keyword");
                        consume![ObjProp];
                    }
                    _ => {}
                }
            }
            GetterSetter if kind == Variable => {
                self.marked = Some("property");
                consume![FUNCTION];
            }
            GetterSetter => retry![AfterProp],
            AfterProp if kind == Punct(':') => consume![Expression(true)],
            AfterProp if kind == Punct('(') => retry![FUNCTION],
            Separated(item, end, step) => {
                let at_end = kind == Punct(end) || value.strip_prefix(end) == Some("");
                let proceed = Separated(item, end, Step::Proceed);
                match step {
                    Step::First if at_end => return true,
                    Step::First => retry![item.rule(), proceed],
                    Step::Proceed if kind == Punct(',') => {
                        consume![Separated(item, end, Step::OptionalItem), proceed]
                    }
                    Step::Proceed if at_end => return true,
                    Step::Proceed => consume![expect(end)],
                    Step::OptionalItem if !at_end => retry![item.rule()],
                    Step::OptionalItem => {}
                }
            }
            Block if kind == Punct('}') => return true,
            Block => retry![Statement, Block],
            MaybeType if ts && kind == Punct(':') => consume![TypeExpr],
            MaybeType if ts && value == "?" => consume![MaybeType],
            MaybeTypeOrIn if ts && (kind == Punct(':') || value == "in") => consume![TypeExpr],
            MaybeRetType if ts && kind == Punct(':') => {
                if re!(format!(r"^{JS_WS}*[A-Za-z0-9_]+{JS_WS}+is(?-u:\b)")).is_match(self.rest()) {
                    consume![EXPR, IsKeyword, TypeExpr];
                }
                consume![TypeExpr];
            }
            IsKeyword if value == "is" => {
                self.marked = Some("keyword");
                return true;
            }
            TypeExpr => {
                if matches!(value, "keyof" | "typeof" | "infer" | "readonly") {
                    self.marked = Some("keyword");
                    consume![if value == "typeof" {
                        Expression(true)
                    } else {
                        TypeExpr
                    }];
                }
                if kind == Variable || value == "void" {
                    self.marked = Some("type");
                    consume![AfterType];
                }
                if matches!(value, "|" | "&") {
                    consume![TypeExpr];
                }
                match kind {
                    String | Number | Atom => consume![AfterType],
                    Punct('[') => list!(TypeExpr, ']', AfterType),
                    Punct('{') => consume![bracket('}'), TypeProps, PopLex, AfterType],
                    Punct('(') => {
                        consume![separated(Item::TypeArg, ')'), MaybeReturnType, AfterType]
                    }
                    Kind::Quasi => retry![Rule::Quasi(true), AfterType],
                    _ => {}
                }
            }
            MaybeReturnType if kind == Arrow => consume![TypeExpr],
            TypeProps if matches!(kind, Punct('}' | ')' | ']')) => return true,
            TypeProps if matches!(kind, Punct(',' | ';')) => consume![TypeProps],
            TypeProps => retry![TypeProp, TypeProps],
            TypeProp => {
                if kind == Variable || style == Some("keyword") {
                    self.marked = Some("property");
                    consume![TypeProp];
                }
                if value == "?" || matches!(kind, Number | String) {
                    consume![TypeProp];
                }
                match kind {
                    Punct(':') => consume![TypeExpr],
                    Punct('[') => consume![Expect(Variable), MaybeTypeOrIn, expect(']'), TypeProp],
                    Punct('(') => retry![DECL, TypeProp],
                    Punct(';' | '}' | ')' | ']' | ',') => {}
                    _ => return true,
                }
            }
            TypeArg
                if kind == Variable
                    && self
                        .rest()
                        .trim_start_matches(js_space)
                        .starts_with(['?', ':'])
                    || value == "?"
                    || kind == Spread =>
            {
                consume![TypeArg]
            }
            TypeArg if kind == Punct(':') => consume![TypeExpr],
            TypeArg => retry![TypeExpr],
            AfterType | MaybeTypeArgs if value == "<" => list!(TypeExpr, '>', AfterType),
            AfterType if matches!(value, "|" | "&") || kind == Punct('.') => consume![TypeExpr],
            AfterType if kind == Punct('[') => consume![TypeExpr, expect(']'), AfterType],
            AfterType if matches!(value, "extends" | "implements") => {
                self.marked = Some("keyword");
                consume![TypeExpr];
            }
            AfterType if value == "?" => consume![TypeExpr, expect(':'), TypeExpr],
            TypeParam => retry![TypeExpr, MaybeTypeDefault],
            MaybeTypeDefault if value == "=" => consume![TypeExpr],
            VarDef if value == "enum" => {
                self.marked = Some("keyword");
                consume![EnumDef];
            }
            VarDef => retry![Pattern, MaybeType, MaybeAssign, VarDefCont],
            Pattern if ts && modifier(value) => {
                self.marked = Some("keyword");
                consume![Pattern];
            }
            Pattern => match kind {
                Variable => {
                    self.register();
                    return true;
                }
                Spread => consume![Pattern],
                Punct('[') => list!(ElementPattern, ']'),
                Punct('{') => list!(PropPattern, '}'),
                _ => {}
            },
            PropPattern => {
                if kind == Variable {
                    if !self.rest().trim_start_matches(js_space).starts_with(':') {
                        self.register();
                        consume![MaybeAssign];
                    }
                    self.marked = Some("property");
                }
                match kind {
                    Spread => consume![Pattern],
                    Punct('}') => {}
                    Punct('[') => consume![EXPR, expect(']'), expect(':'), PropPattern],
                    _ => consume![expect(':'), Pattern, MaybeAssign],
                }
            }
            ElementPattern => retry![Pattern, MaybeAssign],
            MaybeAssign if value == "=" => consume![Expression(true)],
            VarDefCont if kind == Punct(',') => consume![VarDef],
            MaybeElse if kind == KeywordB && value == "else" => {
                consume![PushLex(Lex::Form, "else"), Statement, PopLex]
            }
            ForSpec if value == "await" => consume![ForSpec],
            ForSpec if kind == Punct('(') => consume![bracket(')'), ForSpec1, PopLex],
            ForSpec1 if kind == Var => consume![VarDef, ForSpec2],
            ForSpec1 if kind == Variable => consume![ForSpec2],
            ForSpec1 => retry![ForSpec2],
            ForSpec2 if kind == Punct(')') => return true,
            ForSpec2 if kind == Punct(';') => consume![ForSpec2],
            ForSpec2 if matches!(value, "in" | "of") => {
                self.marked = Some("keyword");
                consume![EXPR, ForSpec2];
            }
            ForSpec2 => retry![EXPR, ForSpec2],
            FunctionDef { body } => {
                if value == "*" {
                    self.marked = Some("keyword");
                    consume![rule];
                }
                if kind == Variable {
                    self.register();
                    consume![rule];
                }
                if kind == Punct('(') {
                    self.push([PopScope]);
                    if body {
                        self.push([Statement]);
                    }
                    consume![
                        FUNCTION_SCOPE,
                        bracket(')'),
                        separated(Item::FunArg, ')'),
                        PopLex,
                        MaybeRetType
                    ];
                }
                if ts && value == "<" {
                    list!(TypeParam, '>', rule);
                }
            }
            TypeName if kind == Variable => {
                self.marked = Some("type");
                consume![TypeName];
            }
            TypeName if value == "<" => list!(TypeParam, '>'),
            FunArg => {
                if value == "@" {
                    self.push([EXPR, FunArg]);
                }
                if kind == Spread {
                    consume![FunArg];
                }
                if ts && modifier(value) {
                    self.marked = Some("keyword");
                    consume![FunArg];
                }
                if ts && kind == This {
                    consume![MaybeType, MaybeAssign];
                }
                retry![Pattern, MaybeType, MaybeAssign];
            }
            ClassExpression if kind != Variable => return self.apply(ClassNameAfter),
            ClassExpression | ClassName if kind == Variable => {
                self.register();
                consume![ClassNameAfter];
            }
            ClassNameAfter if value == "<" => list!(TypeParam, '>', ClassNameAfter),
            ClassNameAfter
                if matches!(value, "extends" | "implements") || ts && kind == Punct(',') =>
            {
                if value == "implements" {
                    self.marked = Some("keyword");
                }
                consume![if ts { TypeExpr } else { EXPR }, ClassNameAfter];
            }
            ClassNameAfter if kind == Punct('{') => consume![bracket('}'), ClassBody, PopLex],
            ClassBody => {
                let rest = self.rest();
                let trimmed = rest.trim_start_matches(js_space);
                if kind == Async
                    || kind == Variable
                        && (matches!(value, "static" | "get" | "set") || ts && modifier(value))
                        && trimmed.len() < rest.len()
                        && trimmed
                            .strip_prefix('#')
                            .unwrap_or(trimmed)
                            .starts_with(word)
                {
                    self.marked = Some("keyword");
                    consume![ClassBody];
                }
                if kind == Variable || style == Some("keyword") {
                    self.marked = Some("property");
                    consume![ClassField, ClassBody];
                }
                match kind {
                    Number | String => consume![ClassField, ClassBody],
                    Punct('[') => consume![EXPR, MaybeType, expect(']'), ClassField, ClassBody],
                    Punct('(') if ts => retry![DECL, ClassBody],
                    Punct(';' | ',') => consume![ClassBody],
                    Punct('}') => return true,
                    _ if value == "*" => {
                        self.marked = Some("keyword");
                        consume![ClassBody];
                    }
                    _ if value == "@" => consume![EXPR, ClassBody],
                    _ => {}
                }
            }
            ClassField => match value {
                "!" | "?" => consume![ClassField],
                "=" => consume![Expression(true)],
                _ if kind == Punct(':') => consume![TypeExpr, MaybeAssign],
                _ => retry![if self
                    .state
                    .lexical
                    .iter()
                    .rev()
                    .nth(1)
                    .is_some_and(|lex| lex.info == "interface")
                {
                    DECL
                } else {
                    FUNCTION
                }],
            },
            AfterExport => match value {
                "*" => {
                    self.marked = Some("keyword");
                    consume![MaybeFrom, expect(';')];
                }
                "default" => {
                    self.marked = Some("keyword");
                    consume![EXPR, expect(';')];
                }
                _ if kind == Punct('{') => {
                    consume![separated(Item::ExportField, '}'), MaybeFrom, expect(';')]
                }
                _ => retry![Statement],
            },
            ExportField if value == "as" => {
                self.marked = Some("keyword");
                consume![Expect(Variable)];
            }
            ExportField if kind == Variable => retry![Expression(true), ExportField],
            AfterImport => match kind {
                String => return true,
                Punct('(') => retry![EXPR],
                Punct('.') => retry![MaybeOperator(false)],
                _ => retry![ImportSpec, MaybeMoreImports, MaybeFrom],
            },
            ImportSpec if kind == Punct('{') => list!(ImportSpec, '}'),
            ImportSpec => {
                if kind == Variable {
                    self.register();
                }
                if value == "*" {
                    self.marked = Some("keyword");
                }
                consume![MaybeAs];
            }
            MaybeMoreImports if kind == Punct(',') => consume![ImportSpec, MaybeMoreImports],
            MaybeAs if value == "as" => {
                self.marked = Some("keyword");
                consume![ImportSpec];
            }
            MaybeFrom if value == "from" => {
                self.marked = Some("keyword");
                consume![EXPR];
            }
            ArrayLiteral if kind == Punct(']') => return true,
            ArrayLiteral => retry![separated(Item::Expression, ']')],
            EnumDef => retry![
                PushLex(Lex::Form, ""),
                Pattern,
                expect('{'),
                bracket('}'),
                separated(Item::EnumMember, '}'),
                PopLex,
                PopLex
            ],
            EnumMember => retry![Pattern, MaybeAssign],
            _ => {}
        }
        false
    }
}

fn modifier(value: &str) -> bool {
    matches!(
        value,
        "public" | "private" | "protected" | "abstract" | "readonly"
    )
}
