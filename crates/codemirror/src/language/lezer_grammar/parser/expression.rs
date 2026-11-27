use super::{Kind::*, Parser, TokenKind};

impl Parser<'_> {
    pub(super) fn body(&mut self) {
        self.delimited(Body, '{', '}', |p| {
            p.expression();
        });
    }

    fn expression(&mut self) -> bool {
        let mark = self.mark();
        let present = self.sequence();
        if !self.at('|') {
            return present;
        }
        while self.take('|') {
            self.sequence();
        }
        self.finish(mark, Choice);
        true
    }

    fn sequence(&mut self) -> bool {
        let start = self.cursor;
        let mut mark = self.mark();
        let mut count = 0;
        let mut marker = false;
        while !(self.boundary() || self.at('|') || self.at(',') && self.closers.contains(&'>')) {
            let item = self.mark();
            if self.take('!') {
                self.require(|p| p.take_name(PrecedenceName));
                self.finish(item, PrecedenceMarker);
                marker = true;
            } else if self.take('~') {
                self.require(|p| p.take_name(Name));
                self.finish(item, AmbiguityMarker);
                marker = true;
            } else if !self.atom() {
                self.unexpected();
                if count == 0 {
                    mark = self.mark();
                }
                continue;
            }
            count += 1;
        }
        if count > 1 || marker {
            self.finish(mark, Sequence);
        }
        self.cursor != start
    }

    fn atom(&mut self) -> bool {
        let mark = self.mark();
        match self.peek() {
            TokenKind::Word => self.named_atom(true),
            TokenKind::Node(kind @ (Literal | CharSet | InvertedCharSet | AnyChar | CharClass)) => {
                self.bump(Some(kind))
            }
            TokenKind::Node(kind @ (AtSpecialize | AtExtend)) => {
                self.bump(Some(kind));
                self.maybe_props();
                self.arguments();
                self.finish(mark, Specialization);
            }
            TokenKind::Node(OpenBracket) => {
                self.props();
                self.body();
                self.finish(mark, InlineRule);
            }
            TokenKind::Node(OpenParen) => {
                self.delimited(ParenExpression, '(', ')', |p| {
                    p.expression();
                });
            }
            _ => return false,
        }
        while let TokenKind::Node(op @ (Question | Star | Plus)) = self.peek() {
            self.bump(Some(op));
            self.finish(
                mark,
                match op {
                    Question => Optional,
                    Star => Repeat,
                    Plus => Repeat1,
                    _ => unreachable!(),
                },
            );
        }
        true
    }

    pub(super) fn name_expression(&mut self) -> bool {
        if self.peek() != TokenKind::Word {
            return false;
        }
        self.named_atom(false);
        true
    }

    fn named_atom(&mut self, inline: bool) {
        let mark = self.mark();
        self.bump(Some(RuleName));
        let scoped = self.take('.');
        if scoped {
            self.require(|p| p.take_name(RuleName));
            self.finish(mark, ScopedName);
        }
        if self.at('<') {
            self.arguments();
            self.finish(mark, Call);
        } else if inline && !scoped && (self.at('[') || self.at('{')) {
            self.maybe_props();
            self.body();
            self.finish(mark, InlineRule);
        }
    }

    fn arguments(&mut self) {
        self.delimited(ArgList, '<', '>', |p| {
            if !p.boundary() {
                p.require(Self::expression);
                while p.take(',') {
                    p.require(Self::expression);
                }
            }
        });
    }
}
