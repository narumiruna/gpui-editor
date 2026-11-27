use super::{Cow, Glr, GlrMode, ParseError, Parser, Recovery, SyntaxTree, TokenCache};

pub(in crate::language) struct RecoveringSession(Option<Glr<'static, 'static>>);

pub(in crate::language) struct Advance {
    pub rounds: usize,
    pub output_work: usize,
    pub output: Option<(SyntaxTree, bool)>,
}

impl RecoveringSession {
    pub(in crate::language) fn new_wast(parser: &'static Parser, source: String) -> Self {
        // WAST has no context hooks to replay during staged reductions.
        assert!(parser.context_tracker.is_none());
        let parse = Glr::new(parser, Cow::Owned(source), GlrMode::WastRecovering);
        parse.tokens.prepare_input();
        Self(Some(parse))
    }

    pub(in crate::language) fn cutoff_after(&self, start: usize, units: usize) -> Option<usize> {
        self.0.as_ref()?.tokens.cutoff_after(start, units)
    }

    pub(in crate::language) fn advance(
        &mut self,
        stop_at: usize,
        rounds: usize,
    ) -> Result<Advance, ParseError> {
        let Some(parse) = &mut self.0 else {
            return Ok(Advance::pending(0));
        };
        for used in 0..=rounds {
            // Do not stop between the two UTF-16 deletion steps of a scalar.
            // Equality includes the token at the cutoff, unless it is real EOF.
            if parse.cursor.byte > stop_at && parse.cursor.at_boundary {
                let (tree, output_work) = parse.stage()?;
                return Ok(Advance {
                    rounds: used,
                    output_work,
                    output: Some((tree, false)),
                });
            }
            if used == rounds {
                break;
            }
            match parse.round() {
                Ok(None) => {}
                Ok(Some(roots)) => {
                    let parse = self.0.take().expect("live frontier");
                    let (tree, output_work) = parse.finish(roots)?;
                    return Ok(Advance {
                        rounds: used + 1,
                        output_work,
                        output: Some((tree, true)),
                    });
                }
                Err(error) => {
                    self.0 = None;
                    return Err(error);
                }
            }
        }
        Ok(Advance::pending(rounds))
    }

    #[cfg(test)]
    pub(in crate::language) fn frontier(&self) -> Option<(usize, bool, usize)> {
        self.0.as_ref().map(|parse| {
            (
                parse.cursor.byte,
                parse.cursor.at_boundary,
                parse.work.remaining(),
            )
        })
    }
}

impl Advance {
    fn pending(rounds: usize) -> Self {
        Self {
            rounds,
            output_work: 0,
            output: None,
        }
    }
}

impl Glr<'_, '_> {
    fn stage(&mut self) -> Result<(SyntaxTree, usize), ParseError> {
        let at = self.cursor.byte;
        let work = self.work.clone();
        let before = work.remaining();
        work.spend(at, self.pending.len())?;
        // Take the highest score at the earliest position, first on a tie.
        let best = self
            .pending
            .iter()
            .rev()
            .filter(|branch| branch.position(branch.stack.pos) == self.cursor)
            .max_by_key(|branch| branch.score)
            .expect("branch at cursor");
        let mut branch = best.fork(&work)?;
        let mut tokens = TokenCache::new(self.tokens.source(), self.parser, true);
        let tree = self.tree.with_staged_output(|tree| {
            Recovery {
                parser: self.parser,
                work: &work,
                tree,
                tokens: &mut tokens,
            }
            .close_at_cutoff(&mut branch)?;
            tree.finish_stage(self.parser, branch.stack.roots, at, &work)
        })?;
        Ok((tree, before - work.remaining()))
    }
}
