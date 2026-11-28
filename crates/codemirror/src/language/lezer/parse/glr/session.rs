use super::{Cow, Glr, GlrMode, ParseError, Parser, Recovery, SyntaxTree, TokenCache};

pub(in crate::language) struct RecoveringSession(Option<Glr<'static, 'static>>);

pub(in crate::language) struct Advance {
    pub rounds: usize,
    pub output_work: usize,
    pub output: Option<(SyntaxTree, bool)>,
}

impl RecoveringSession {
    pub(in crate::language) fn new_wast(parser: &'static Parser, source: String) -> Self {
        Self::new(parser, source, GlrMode::WastRecovering)
    }

    pub(in crate::language) fn new_json(parser: &'static Parser, source: String) -> Self {
        Self::new(parser, source, GlrMode::JsonRecovering)
    }

    pub(in crate::language) fn new_lezer(parser: &'static Parser, source: String) -> Self {
        Self::new(parser, source, GlrMode::Recovering)
    }

    pub(in crate::language) fn new_strict_lezer(parser: &'static Parser, source: String) -> Self {
        Self::new(parser, source, GlrMode::EditorStrict)
    }

    fn new(parser: &'static Parser, source: String, mode: GlrMode) -> Self {
        // Staged reductions must not replay context hooks, and live token
        // specialization must stay bounded and independent of other jobs.
        assert!(parser.context_tracker.is_none());
        assert!(
            parser
                .specializers
                .iter()
                .all(|hook| hook.is_context_free())
        );
        let parse = Glr::new(parser, Cow::Owned(source), mode);
        parse.tokens.prepare_input();
        Self(Some(parse))
    }

    pub(in crate::language) fn cutoff_after(&self, start: usize, units: usize) -> Option<usize> {
        self.0.as_ref()?.tokens.cutoff_after(start, units)
    }

    pub(in crate::language) fn position(&self) -> Option<usize> {
        Some(self.0.as_ref()?.cursor.byte)
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
            // Equality includes the token at the cutoff, unless it is real EOF.
            // The pinned cutoff can stop at the first UTF-16 deletion step
            // of an astral scalar. Its output range already spans that scalar.
            if parse.cursor.byte > stop_at
                && (parse.cursor.at_boundary || parse.mode != GlrMode::WastRecovering)
            {
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
        let strict = self.mode == GlrMode::EditorStrict;
        // After all stacks pass the cutoff, the pinned strict parser discards
        // stopped stacks while any stack is still ahead. No tokens are read.
        let end = if strict {
            self.pending
                .iter()
                .map(|branch| branch.position(branch.stack.pos))
                .max()
                .expect("live frontier")
        } else {
            self.cursor
        };
        let at = end.byte;
        let work = self.work.clone();
        let before = work.remaining();
        work.spend(at, self.pending.len())?;
        let mut tokens = TokenCache::new(self.tokens.source(), self.parser, !strict);
        let tree = self
            .tree
            .with_staged_output(self.parser, at, &work, |tree| {
                let mut recovery = Recovery {
                    parser: self.parser,
                    work: &work,
                    tree,
                    tokens: &mut tokens,
                    // Closing a staged copy must not affect the live parse.
                    large_reductions: None,
                };
                let at_end = |branch: &super::Branch| branch.position(branch.stack.pos) == end;
                let mut branches = self.pending.iter().filter(|branch| at_end(branch));
                let branch = if self.mode != GlrMode::WastRecovering {
                    let mut close = |branch: &mut super::Branch| {
                        if strict {
                            recovery.force_close_at_cutoff(branch)
                        } else {
                            recovery.close_at_cutoff(branch)
                        }
                    };
                    let mut best = branches.next().expect("branch at stage end").fork(&work)?;
                    let mut accepting = close(&mut best)?;
                    // When every stack stops here, prefer the best accepting stack
                    // after forced reductions. Otherwise the pin finishes the first
                    // stopped recovery stack without comparing scores.
                    if strict || self.pending.iter().all(at_end) {
                        for branch in branches {
                            let mut candidate = branch.fork(&work)?;
                            if close(&mut candidate)?
                                && (!accepting || candidate.score > best.score)
                            {
                                best = candidate;
                                accepting = true;
                            }
                        }
                    }
                    if strict && !accepting {
                        return Err(best.stack.unexpected_token(None));
                    }
                    best
                } else {
                    // Preserve WAST's highest-score selection, first on a tie.
                    let mut best = branches
                        .rev()
                        .max_by_key(|branch| branch.score)
                        .expect("branch at cursor")
                        .fork(&work)?;
                    recovery.close_at_cutoff(&mut best)?;
                    best
                };
                Ok(branch.stack.roots)
            })?;
        Ok((tree, before - work.remaining()))
    }
}
