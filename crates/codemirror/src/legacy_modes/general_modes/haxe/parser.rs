use super::{Kind, word};

#[derive(Clone, Debug)]
pub(super) struct State {
    pending: Vec<Action>,
    lexical: Vec<Lexical>,
    start_unit: usize,
    indented: usize,
    locals: Vec<String>,
    scopes: Vec<usize>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LexKind {
    Root,
    Stat,
    Form,
    VarDef,
    Close(char),
    Switch,
}

#[derive(Clone, Copy, Debug)]
struct Lexical {
    kind: LexKind,
    indented: usize,
    column: usize,
    align: Option<bool>,
}

use Action::*;
use Kind::{Punct, Type, Variable};
use LexKind::{Close, Form, Stat, VarDef};

#[derive(Clone, Copy, Debug)]
enum Action {
    Statement,
    Expression,
    MaybeExpression,
    MaybeOperator,
    MaybeAttribute,
    ImportDef,
    TypeDef,
    MaybeLabel,
    Property,
    ObjectProp,
    CommaList(Item, char),
    CommaNext(Item, char),
    Block,
    VarName,
    VarInit,
    ForSpec,
    ForIn,
    FunctionDef,
    TypeUse,
    TypeString,
    TypeProp,
    FunArg,
    Expect(char),
    PushLex(LexKind),
    PopLex,
    PushScope,
    PopScope,
}

#[derive(Clone, Copy, Debug)]
enum Item {
    OptionalExpression,
    Expression,
    ObjectProp,
    TypeProp,
    FunArg,
}

impl Item {
    fn action(self) -> Action {
        match self {
            Self::OptionalExpression => MaybeExpression,
            Self::Expression => Expression,
            Self::ObjectProp => ObjectProp,
            Self::TypeProp => TypeProp,
            Self::FunArg => FunArg,
        }
    }
}

impl State {
    pub(super) fn new(start_unit: usize) -> Self {
        Self {
            pending: Vec::new(),
            lexical: vec![Lexical {
                kind: LexKind::Root,
                indented: 0,
                column: 0,
                align: Some(false),
            }],
            start_unit,
            indented: 0,
            locals: Vec::new(),
            scopes: Vec::new(),
        }
    }

    pub(super) fn start_line(&mut self, indentation: usize) {
        self.lexical.last_mut().unwrap().align.get_or_insert(false);
        self.indented = indentation;
    }

    pub(super) fn parse(&mut self, kind: Kind, text: &str, column: usize) -> Option<&'static str> {
        self.lexical.last_mut().unwrap().align.get_or_insert(true);
        let mut marked = None;
        loop {
            let action = self.pending.pop().unwrap_or(Statement);
            if self.step(action, kind, text, column, &mut marked) {
                break;
            }
        }
        // Lexical actions run immediately after a token, even across line breaks.
        while let Some(&action @ (PushLex(_) | PopLex | PopScope)) = self.pending.last() {
            self.pending.pop();
            self.lex_action(action, column);
        }
        marked.or_else(|| {
            (kind == Variable && self.locals.iter().any(|name| name == text))
                .then_some("variableName.local")
        })
    }

    pub(super) fn indent(&self, text_after: &str, unit: usize) -> usize {
        let first = text_after.chars().next();
        let mut depth = self.lexical.len() - 1;
        if self.lexical[depth].kind == Stat && first == Some('}') {
            depth -= 1;
        }
        let context = self.lexical[depth];
        let indent = context.indented;
        match context.kind {
            LexKind::Root => unit.saturating_sub(self.start_unit),
            VarDef => indent.saturating_add(4),
            Form if first == Some('{') => indent,
            Stat | Form => indent.saturating_add(unit),
            LexKind::Switch | Close(_) => {
                let closing = first
                    == Some(match context.kind {
                        Close(ch) => ch,
                        _ => '}',
                    });
                if context.kind == LexKind::Switch && !closing {
                    if ["case", "default"].iter().any(|prefix| {
                        text_after
                            .strip_prefix(prefix)
                            .is_some_and(|tail| !tail.chars().next().is_some_and(word))
                    }) {
                        indent.saturating_add(unit)
                    } else {
                        indent.saturating_add(unit.saturating_mul(2))
                    }
                } else if context.align == Some(true) {
                    context.column.saturating_add(usize::from(!closing))
                } else {
                    indent.saturating_add(if closing { 0 } else { unit })
                }
            }
        }
    }

    fn push(&mut self, actions: &[Action]) -> bool {
        self.pending.extend(actions.iter().rev().copied());
        true
    }

    fn pass(&mut self, actions: &[Action]) -> bool {
        self.push(actions);
        false
    }

    fn lex_action(&mut self, action: Action, column: usize) {
        match action {
            PushLex(kind) => self.lexical.push(Lexical {
                kind,
                indented: self.indented,
                column,
                align: None,
            }),
            PopLex if self.lexical.len() > 1 => {
                let prev = self.lexical.pop().unwrap();
                if prev.kind == Close(')') {
                    self.indented = prev.indented;
                }
            }
            PopScope => self.locals.truncate(self.scopes.pop().unwrap()),
            PopLex => {}
            _ => unreachable!(),
        }
    }

    fn register(&mut self, text: &str, marked: &mut Option<&'static str>) {
        if !self.scopes.is_empty() {
            *marked = Some("def");
            if !self.locals.iter().any(|name| name == text) {
                self.locals.push(text.to_owned());
            }
        }
    }

    fn step(
        &mut self,
        action: Action,
        kind: Kind,
        text: &str,
        column: usize,
        marked: &mut Option<&'static str>,
    ) -> bool {
        match action {
            Statement => self.statement(kind),
            Expression => self.expression(kind),
            MaybeOperator => self.maybe_operator(kind, text),
            MaybeExpression if matches!(kind, Punct(';' | '}' | ')' | ']' | ',')) => false,
            MaybeExpression => self.pass(&[Expression]),
            MaybeAttribute => match kind {
                Kind::Attribute => self.push(&[MaybeAttribute]),
                Kind::Function => self.push(&[FunctionDef]),
                Kind::Var => self.push(&[VarName]),
                _ => false,
            },
            // Uppercase names are Type tokens, so the upstream import
            // registration branch for uppercase Variable tokens is unreachable.
            ImportDef if matches!(kind, Variable | Punct('.')) || text == "*" => {
                self.push(&[ImportDef])
            }
            TypeDef => kind == Type,
            MaybeLabel if kind == Punct(':') => self.push(&[PopLex, Statement]),
            MaybeLabel => self.pass(&[MaybeOperator, Expect(';'), PopLex]),
            Property if kind == Variable => {
                *marked = Some("property");
                true
            }
            ObjectProp if kind.atomic() => {
                if kind == Variable {
                    *marked = Some("property");
                }
                self.push(&[Expect(':'), Expression])
            }
            CommaList(item, end) => {
                kind == Punct(end) || self.pass(&[item.action(), CommaNext(item, end)])
            }
            CommaNext(item, end) if kind == Punct(',') => {
                self.push(&[item.action(), CommaNext(item, end)])
            }
            CommaNext(_, end) => kind == Punct(end) || self.push(&[Expect(end)]),
            Block => kind == Punct('}') || self.pass(&[Statement, Block]),
            VarName if kind == Variable => {
                self.register(text, marked);
                self.push(&[TypeUse, VarInit])
            }
            VarName => true,
            VarInit if text == "=" => self.push(&[Expression, VarInit]),
            VarInit if kind == Punct(',') => self.push(&[VarName]),
            ForSpec if kind == Variable => {
                self.register(text, marked);
                self.push(&[ForIn, Expression])
            }
            ForIn => text == "in",
            FunctionDef if matches!(kind, Variable | Type) => {
                self.register(text, marked);
                self.push(&[FunctionDef])
            }
            FunctionDef if text == "new" => self.push(&[FunctionDef]),
            FunctionDef if kind == Punct('(') => self.push(&[
                PushLex(Close(')')),
                PushScope,
                CommaList(Item::FunArg, ')'),
                PopLex,
                TypeUse,
                Statement,
                PopScope,
            ]),
            TypeUse if kind == Punct(':') => self.push(&[TypeString]),
            TypeString if matches!(kind, Type | Variable) => true,
            TypeString if kind == Punct('{') => {
                self.push(&[PushLex(Close('}')), CommaList(Item::TypeProp, '}'), PopLex])
            }
            TypeProp if kind == Variable => self.push(&[TypeUse]),
            FunArg if kind == Variable => {
                self.register(text, marked);
                self.push(&[TypeUse])
            }
            Expect(ch) => kind == Punct(ch) || ch != ';' && self.push(&[Expect(ch)]),
            PushLex(_) | PopLex | PopScope => {
                self.lex_action(action, column);
                false
            }
            PushScope => {
                if self.scopes.is_empty() {
                    self.locals.clear();
                    self.locals.push("this".into());
                }
                self.scopes.push(self.locals.len());
                false
            }
            _ => false,
        }
    }

    fn statement(&mut self, kind: Kind) -> bool {
        match kind {
            Kind::Var => self.push(&[PushLex(VarDef), VarName, Expect(';'), PopLex]),
            Kind::Conditional => self.push(&[PushLex(Form), Expression, Statement, PopLex]),
            Kind::Form => self.push(&[PushLex(Form), Statement, PopLex]),
            Punct('{') => self.push(&[PushLex(Close('}')), PushScope, Block, PopLex, PopScope]),
            Punct(';') => true,
            Kind::Attribute => self.push(&[MaybeAttribute]),
            Kind::Function => self.push(&[FunctionDef]),
            Kind::For => self.push(&[
                PushLex(Form),
                Expect('('),
                PushLex(Close(')')),
                ForSpec,
                Expect(')'),
                PopLex,
                Statement,
                PopLex,
            ]),
            Variable => self.push(&[PushLex(Stat), MaybeLabel]),
            Kind::Switch => self.push(&[
                PushLex(Form),
                Expression,
                PushLex(LexKind::Switch),
                Expect('{'),
                Block,
                PopLex,
                PopLex,
            ]),
            Kind::Case => self.push(&[Expression, Expect(':')]),
            Kind::Default => self.push(&[Expect(':')]),
            Kind::Catch => self.push(&[
                PushLex(Form),
                PushScope,
                Expect('('),
                FunArg,
                Expect(')'),
                Statement,
                PopLex,
                PopScope,
            ]),
            Kind::Import => self.push(&[ImportDef, Expect(';')]),
            Kind::TypeDef => self.push(&[TypeDef]),
            _ => self.pass(&[PushLex(Stat), Expression, Expect(';'), PopLex]),
        }
    }

    fn expression(&mut self, kind: Kind) -> bool {
        match kind {
            kind if kind.atomic() || kind == Type => self.push(&[MaybeOperator]),
            Kind::Function => self.push(&[FunctionDef]),
            Kind::Prefix => self.push(&[MaybeExpression]),
            Punct('(') => self.push(&[
                PushLex(Close(')')),
                MaybeExpression,
                Expect(')'),
                PopLex,
                MaybeOperator,
            ]),
            Kind::Operator => self.push(&[Expression]),
            Punct('[') => self.push(&[
                PushLex(Close(']')),
                CommaList(Item::OptionalExpression, ']'),
                PopLex,
                MaybeOperator,
            ]),
            Punct('{') => self.push(&[
                PushLex(Close('}')),
                CommaList(Item::ObjectProp, '}'),
                PopLex,
                MaybeOperator,
            ]),
            _ => true,
        }
    }

    fn maybe_operator(&mut self, kind: Kind, text: &str) -> bool {
        match kind {
            Kind::Operator if text.contains("++") || text.contains("--") => {
                self.push(&[MaybeOperator])
            }
            Kind::Operator | Punct(':') => self.push(&[Expression]),
            Punct('(') => self.push(&[
                PushLex(Close(')')),
                CommaList(Item::Expression, ')'),
                PopLex,
                MaybeOperator,
            ]),
            Punct('.') => self.push(&[Property, MaybeOperator]),
            Punct('[') => self.push(&[
                PushLex(Close(']')),
                Expression,
                Expect(']'),
                PopLex,
                MaybeOperator,
            ]),
            _ => false,
        }
    }
}
