use std::sync::LazyLock;

use regex::Regex;

use super::{StringStream, word};

const KEYWORDS: &str = "return case switch else elseif end endif endfunction if otherwise do for while try catch classdef properties events methods global persistent endfor endwhile printf sprintf disp until continue pkg";
const BUILTINS: &str = "error eval function abs acos atan asin cos cosh exp log prod sum log10 max min sign sin sinh sqrt tan reshape break zeros default margin round ones rand syn ceil floor size clear eye mean std cov det eig inv norm rank trace expm logm sqrtm linspace plot title xlabel ylabel legend text grid meshgrid mesh num2str fft ifft arrayfun cellfun input fliplr flipud ismember";

static NUMBER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:[+-]?0x[0-9a-fA-F]+[ij]?|[+-]?[0-9]*\.[0-9]+(?:[EeDd][+-]?[0-9]+)?[ij]?|[+-]?[0-9]+(?:[EeDd][+-]?[0-9]+)?[ij]?)")
        .expect("valid Octave number pattern")
});
static STRING: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"^(?:"(?:[^"]|"")*("|$)|'(?:[^']|'')*('|$))"#)
        .expect("valid Octave string pattern")
});
static COMMENT_END: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[^\r\n\u{2028}\u{2029}]*%\}").expect("valid Octave comment pattern")
});

#[derive(Clone, Debug, Default)]
pub(super) enum State {
    #[default]
    Base,
    Comment,
    Transpose,
}

impl State {
    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        if matches!(self, Self::Comment) {
            if stream.match_regex(&COMMENT_END, true).is_some() {
                *self = Self::Base;
            } else {
                stream.skip_to_end();
            }
            return Some("comment");
        }
        if matches!(self, Self::Transpose) {
            *self = Self::Base;
            if !stream.sol() && stream.eat('\'').is_some() {
                return Some("operator");
            }
        }
        let style = self.base(stream);
        if matches!(style, Some("number" | "variable")) {
            *self = Self::Transpose;
        }
        style
    }

    fn base(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        if stream.eat_space() || stream.eol() {
            return None;
        }
        if stream.match_str("%{", true, false) {
            *self = Self::Comment;
            stream.skip_to_end();
            return Some("comment");
        }
        if stream.eat_if(|ch| matches!(ch, '%' | '#')).is_some() {
            stream.skip_to_end();
            return Some("comment");
        }
        if stream.match_regex(&NUMBER, true).is_some() {
            return Some("number");
        }
        if let Some(matched) = stream.match_regex(&STRING, true) {
            let closed = [1, 2]
                .into_iter()
                .any(|group| matched.get(group).is_some_and(|m| !m.is_empty()));
            return Some(if closed { "string" } else { "error" });
        }
        let text = &stream.line()[stream.pos()..];
        let word_end = text.find(|ch| !word(ch)).unwrap_or(text.len());
        let word = &text[..word_end];
        let style = if matches!(word, "nan" | "NaN" | "inf" | "Inf") {
            Some("number")
        } else if KEYWORDS.split_ascii_whitespace().any(|item| item == word) {
            Some("keyword")
        } else if BUILTINS.split_ascii_whitespace().any(|item| item == word) {
            Some("builtin")
        } else {
            None
        };
        if style.is_some() {
            stream.set_pos(stream.pos() + word_end).unwrap();
            return style;
        }
        let identifier_start = |ch: char| ch.is_ascii_alphabetic() || ch == '_' || ch >= '\u{a1}';
        if stream.eat_if(identifier_start).is_some() {
            stream.eat_while(|ch| identifier_start(ch) || ch.is_ascii_digit());
            return Some("variable");
        }
        if stream.eat_if(|ch| "+-*/&|^~<>!@'\\".contains(ch)).is_some()
            || [
                "==", "~=", "<=", ">=", "<<", ">>", ".+", ".-", ".*", "./", ".^", ".\\",
            ]
            .into_iter()
            .any(|op| stream.match_str(op, true, false))
        {
            return Some("operator");
        }
        // Single operators have already taken the first character of compound assignments.
        if stream.eat_if(|ch| "([{},:=;.".contains(ch)).is_some() {
            return None;
        }
        if stream.eat_if(|ch| matches!(ch, ']' | ')')).is_some() {
            *self = Self::Transpose;
            return None;
        }
        stream.next();
        Some("error")
    }
}
