use std::sync::LazyLock;

use regex::Regex;

use super::{StringStream, close_string, word};
use crate::legacy_modes::js_space;

const KEYWORDS: &str = "abs acos aj aj0 all and any asc asin asof atan attr avg avgs bin by ceiling cols cor cos count cov cross csv cut delete deltas desc dev differ distinct div do each ej enlist eval except exec exit exp fby fills first fkeys flip floor from get getenv group gtime hclose hcount hdel hopen hsym iasc idesc if ij in insert inter inv key keys last like list lj load log lower lsq ltime ltrim mavg max maxs mcount md5 mdev med meta min mins mmax mmin mmu mod msum neg next not null or over parse peach pj plist prd prds prev prior rand rank ratios raze read0 read1 reciprocal reverse rload rotate rsave rtrim save scan select set setenv show signum sin sqrt ss ssr string sublist sum sums sv system tables tan til trim txf type uj ungroup union update upper upsert value var view views vs wavg where while within wj wj1 wsum xasc xbar xcol xcols xdesc xexp xgroup xkey xlog xprev xrank";
const SEPARATORS: &str = "|/&^!+:\\-*%$=~#;@><,?_'\"[()]{}";

static TEMPORAL: LazyLock<Regex> = LazyLock::new(|| {
    let clock = r"[0-9]{2}(?::[0-9]{2}(?::[0-9]{2}(?:\.[0-9]{1,9})?)?)?";
    Regex::new(&format!(r"^(?:[0-9]{{4}}\.[0-9]{{2}}(?:m|\.[0-9]{{2}}(?:[DT](?:{clock})?)?)|[0-9]+D{clock}|[0-9]{{2}}:[0-9]{{2}}(?::[0-9]{{2}}(?:\.[0-9]{{1,9}})?)?|[0-9]+[ptuv])"))
        .expect("valid q temporal pattern")
});
static NUMBER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:0[NwW]|0x[0-9a-fA-F]*|[01]+b|[0-9]+[chijn]|-?[0-9]*(?:\.[0-9]*)?(?:e[+-]?[0-9]+)?[ef]?)")
        .expect("valid q number pattern")
});

#[derive(Clone, Debug, Default)]
pub(super) enum State {
    #[default]
    Base,
    String,
    Comment,
    CommentToEnd,
}

impl State {
    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        match self {
            Self::String => return Some(self.string(stream)),
            Self::Comment => {
                if stream.sol() && stream.eat('\\').is_some() && only_space_left(stream) {
                    *self = Self::Base;
                }
                stream.skip_to_end();
                return Some("comment");
            }
            Self::CommentToEnd => {
                stream.skip_to_end();
                return Some("comment");
            }
            Self::Base => {}
        }

        let sol = stream.sol();
        let ch = stream.next()?;
        match ch {
            '/' if sol => {
                if only_space_left(stream) {
                    *self = Self::Comment;
                }
                stream.skip_to_end();
                Some("comment")
            }
            '\\' if sol => {
                if stream.eol() || stream.peek().is_some_and(js_space) {
                    if only_space_left(stream) {
                        *self = Self::CommentToEnd;
                    }
                    stream.skip_to_end();
                    Some("comment")
                } else {
                    Some("builtin")
                }
            }
            ch if js_space(ch) => {
                if stream.peek() == Some('/') {
                    stream.skip_to_end();
                    Some("comment")
                } else {
                    Some("null")
                }
            }
            '"' => Some(self.string(stream)),
            '`' => {
                stream.eat_while(|ch| word(ch) || matches!(ch, ':' | '/' | '.'));
                Some("macroName")
            }
            ch if ch.is_ascii_digit()
                || (ch == '.' && stream.peek().is_some_and(|ch| ch.is_ascii_digit())) =>
            {
                stream.back_up(1).unwrap();
                let style = if stream.match_regex(&TEMPORAL, true).is_some() {
                    "temporal"
                } else {
                    stream.match_regex(&NUMBER, true);
                    "number"
                };
                if stream
                    .peek()
                    .is_none_or(|ch| js_space(ch) || SEPARATORS.contains(ch))
                {
                    Some(style)
                } else {
                    stream.next();
                    Some("error")
                }
            }
            ch if ch.is_ascii_alphabetic() || ch == '.' => {
                stream.eat_while(|ch| word(ch) || ch == '.');
                Some(
                    if KEYWORDS
                        .split_ascii_whitespace()
                        .any(|item| item == stream.current())
                    {
                        "keyword"
                    } else {
                        "variable"
                    },
                )
            }
            ch if SEPARATORS.contains(ch) => None,
            _ => Some("error"),
        }
    }

    fn string(&mut self, stream: &mut StringStream<'_>) -> &'static str {
        *self = if close_string(stream) {
            Self::Base
        } else {
            Self::String
        };
        "string"
    }
}

fn only_space_left(stream: &StringStream<'_>) -> bool {
    stream.line()[stream.pos()..].chars().all(js_space)
}
