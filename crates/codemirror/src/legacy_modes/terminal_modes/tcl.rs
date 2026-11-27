use std::{collections::HashSet, sync::LazyLock};

use super::{
    FUNCTIONS, Params, ends_comment, ends_region, number_tail, operator, starts_region, word,
};
use crate::stream_parser::StringStream;

#[derive(Clone, Debug, Default)]
pub(super) struct State {
    phase: Phase,
    params: Params,
}

#[derive(Clone, Copy, Debug, Default)]
enum Phase {
    #[default]
    Base,
    String(char),
    Comment,
    Region,
}

impl State {
    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        if stream.eat_space() {
            return None;
        }
        self.scan(stream)
    }

    fn scan(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        match self.phase {
            Phase::Base => self.base(stream),
            Phase::String(quote) => {
                let mut escaped = false;
                for ch in stream {
                    if ch == quote && !escaped {
                        self.phase = Phase::Base;
                        break;
                    }
                    escaped = !escaped && ch == '\\';
                }
                Some("string")
            }
            Phase::Comment => {
                if ends_comment(stream, '#') {
                    self.phase = Phase::Base;
                }
                Some("comment")
            }
            Phase::Region => {
                if ends_region(stream, ']', '#') {
                    self.phase = Phase::Base;
                }
                Some("meta")
            }
        }
    }

    fn base(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        static COMMANDS: LazyLock<HashSet<&str>> =
            LazyLock::new(|| KEYWORDS.split_ascii_whitespace().collect());

        let before = std::mem::take(&mut self.params.before);
        let ch = stream.next()?;
        match ch {
            '"' | '\'' if self.params.inside => {
                self.phase = Phase::String(ch);
                self.scan(stream)
            }
            '[' | ']' | '{' | '}' | '(' | ')' | ',' | ';' | '.' => {
                self.params.punctuation(ch, before);
                None
            }
            '0'..='9' => {
                stream.eat_while(number_tail);
                Some("number")
            }
            '#' if stream.eat('*').is_some() => {
                self.phase = Phase::Comment;
                self.scan(stream)
            }
            '#' if starts_region(stream, '[') => {
                self.phase = Phase::Region;
                self.scan(stream)
            }
            '#' => {
                stream.skip_to_end();
                Some("comment")
            }
            '"' => {
                // Upstream passes a RegExp to skipTo, which searches for its literal text.
                stream.skip_to("/\"/");
                Some("comment")
            }
            '$' => {
                stream.eat_while(|ch| word(ch) || "$.{:".contains(ch));
                stream.eat_while(|ch| ch == '}');
                self.params.before = true;
                Some("builtin")
            }
            ch if operator(ch) => {
                stream.eat_while(operator);
                Some("comment")
            }
            _ => {
                stream.eat_while(|ch| word(ch) || "${}".contains(ch) || ch >= '\u{a1}');
                let command = stream.current().to_lowercase();
                if COMMANDS.contains(command.as_str()) {
                    Some("keyword")
                } else if FUNCTIONS.contains(&command.as_str()) {
                    self.params.before = true;
                    Some("keyword")
                } else {
                    None
                }
            }
        }
    }
}

const KEYWORDS: &str = "\
    Tcl safe after append array auto_execok auto_import auto_load \
    auto_mkindex auto_mkindex_old auto_qualify auto_reset bgerror \
    binary break catch cd close concat continue dde eof encoding error \
    eval exec exit expr fblocked fconfigure fcopy file fileevent filename \
    filename flush for foreach format gets glob global history http if \
    incr info interp join lappend lindex linsert list llength load lrange \
    lreplace lsearch lset lsort memory msgcat namespace open package parray \
    pid pkg::create pkg_mkIndex proc puts pwd re_syntax read regex regexp \
    registry regsub rename resource return scan seek set socket source split \
    string subst switch tcl_endOfWord tcl_findLibrary tcl_startOfNextWord \
    tcl_wordBreakAfter tcl_startOfPreviousWord tcl_wordBreakBefore tcltest \
    tclvars tell time trace unknown unset update uplevel upvar variable \
    vwait";
