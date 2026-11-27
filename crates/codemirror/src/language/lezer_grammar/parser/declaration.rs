use super::{Kind, Kind::*, Parser, TokenKind};

impl Parser<'_> {
    pub(super) fn declaration(&mut self) {
        let mark = self.mark();
        let kind = match self.peek() {
            TokenKind::Word | TokenKind::Node(AtTop) => return self.rule(),
            TokenKind::Node(
                kind @ (AtPrecedence | AtTokens | AtLocal | AtExternal | AtContext | AtDialects
                | AtSkip | AtDetectDelim),
            ) => kind,
            _ => return self.unexpected(),
        };
        self.bump(Some(kind));
        let kind = match kind {
            AtPrecedence => {
                self.delimited(PrecedenceBody, '{', '}', |p| {
                    while !p.boundary() {
                        let mark = p.mark();
                        if p.take_name(PrecedenceName) {
                            if let TokenKind::Node(kind @ (AtLeft | AtRight | AtCut)) = p.peek() {
                                p.bump(Some(kind));
                            }
                            p.finish(mark, Precedence);
                            p.take(',');
                        } else {
                            p.unexpected();
                        }
                    }
                });
                PrecedenceDeclaration
            }
            AtTokens => {
                self.tokens_body(false);
                TokensDeclaration
            }
            AtLocal => {
                self.require(|p| p.keyword(Tokens));
                self.tokens_body(true);
                LocalTokensDeclaration
            }
            AtExternal => self.external(),
            AtContext => {
                self.require(|p| p.take_name(Name));
                self.module_path();
                ContextDeclaration
            }
            AtDialects => {
                self.delimited(DialectBody, '{', '}', |p| {
                    while !p.boundary() {
                        if p.take_name(Name) {
                            p.take(',');
                        } else {
                            p.unexpected();
                        }
                    }
                });
                DialectsDeclaration
            }
            AtSkip => {
                self.body();
                if self.at('{') {
                    self.delimited(SkipBody, '{', '}', |p| {
                        while !p.boundary() {
                            if p.peek() == TokenKind::Word || p.at_node(AtTop) {
                                p.rule();
                            } else {
                                p.unexpected();
                            }
                        }
                    });
                    SkipScope
                } else {
                    TopSkipDeclaration
                }
            }
            AtDetectDelim => DetectDelimDeclaration,
            _ => unreachable!(),
        };
        self.finish(mark, kind);
    }

    fn rule(&mut self) {
        let mark = self.mark();
        self.take_node(AtTop);
        let named = self.take_name(RuleName);
        if !named {
            self.missing();
        }
        if named || self.at('[') || self.at('<') || self.at('{') {
            self.maybe_props();
            if self.at('<') {
                self.delimited(ParamList, '<', '>', |p| {
                    if p.take_name(Name) {
                        while p.take(',') {
                            p.require(|p| p.take_name(Name));
                        }
                    }
                    while !p.boundary() && !p.at('{') {
                        p.unexpected();
                    }
                });
            }
            self.body();
        }
        self.finish(mark, RuleDeclaration);
    }

    fn module_path(&mut self) {
        self.require(|p| p.keyword(From));
        self.require(|p| p.take_node(Literal));
    }

    fn external(&mut self) -> Kind {
        if self.keyword(PropKeyword) {
            self.require(|p| p.take_name(Name));
            if self.keyword(As) {
                self.require(|p| p.take_name(Name));
            }
            self.module_path();
            ExternalPropDeclaration
        } else if self.keyword(PropSource) {
            self.require(|p| p.take_name(Name));
            self.module_path();
            ExternalPropSourceDeclaration
        } else if self.keyword(Tokens) {
            self.require(|p| p.take_name(Name));
            self.module_path();
            self.external_token_set();
            ExternalTokensDeclaration
        } else if self.keyword(Extend) || self.keyword(Specialize) {
            self.body();
            self.require(|p| p.take_name(Name));
            self.module_path();
            self.external_token_set();
            ExternalSpecializeDeclaration
        } else {
            self.missing();
            Error
        }
    }

    fn external_token_set(&mut self) {
        self.delimited(TokensBody, '{', '}', |p| {
            while !p.boundary() {
                let mark = p.mark();
                if p.take_name(RuleName) {
                    p.maybe_props();
                    p.finish(mark, Token);
                    p.take(',');
                } else if p.take_node(AtConflict) {
                    p.delimited(ConflictBody, '{', '}', |p| {
                        while !p.boundary() {
                            if p.name_expression() {
                                p.take(',');
                            } else {
                                p.unexpected();
                            }
                        }
                    });
                    p.finish(mark, ExternalConflictDeclaration);
                } else {
                    p.unexpected();
                    continue;
                }
                p.take(',');
            }
        });
    }

    fn tokens_body(&mut self, local: bool) {
        self.delimited(TokensBody, '{', '}', |p| {
            while !p.boundary() {
                let mark = p.mark();
                let kind = match p.peek() {
                    TokenKind::Word => {
                        p.rule();
                        continue;
                    }
                    TokenKind::Node(AtPrecedence) => {
                        p.bump(Some(AtPrecedence));
                        p.delimited(PrecedenceBody, '{', '}', |p| {
                            while !p.boundary() {
                                if p.take_node(Literal) || p.name_expression() {
                                    p.take(',');
                                } else {
                                    p.unexpected();
                                }
                            }
                        });
                        TokenPrecedenceDeclaration
                    }
                    TokenKind::Node(AtConflict) => {
                        p.bump(Some(AtConflict));
                        p.delimited(ConflictBody, '{', '}', |p| {
                            p.require(|p| p.take_node(Literal) || p.name_expression());
                            p.take(',');
                            p.require(|p| p.take_node(Literal) || p.name_expression());
                            while !p.boundary() {
                                p.unexpected();
                            }
                        });
                        TokenConflictDeclaration
                    }
                    TokenKind::Node(Literal) => {
                        p.bump(Some(Literal));
                        p.maybe_props();
                        LiteralTokenDeclaration
                    }
                    TokenKind::Node(AtElse) if local => {
                        p.bump(Some(AtElse));
                        p.require(|p| p.take_name(RuleName));
                        p.maybe_props();
                        ElseToken
                    }
                    _ => {
                        p.unexpected();
                        continue;
                    }
                };
                p.finish(mark, kind);
            }
        });
    }

    pub(super) fn maybe_props(&mut self) {
        if self.at('[') {
            self.props();
        }
    }

    pub(super) fn props(&mut self) {
        self.delimited(Props, '[', ']', |p| {
            while !p.boundary() && !p.at('{') {
                let mark = p.mark();
                if !p.take_name(Name) && !p.take_node(AtName) {
                    p.unexpected();
                    continue;
                }
                if p.take('=') {
                    loop {
                        if p.at('{') {
                            p.delimited(PropEsc, '{', '}', |p| {
                                p.require(|p| p.take_name(RuleName))
                            });
                        } else if !(p.take_node(Literal) || p.take_name(Name) || p.take('.')) {
                            break;
                        }
                    }
                }
                p.finish(mark, Prop);
                if p.take(',') {
                    if p.boundary() {
                        p.missing();
                    }
                } else if !p.boundary() && !p.at('{') {
                    p.missing();
                }
            }
        });
    }
}
