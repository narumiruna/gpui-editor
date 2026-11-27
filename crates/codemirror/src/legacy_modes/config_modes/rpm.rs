use std::sync::LazyLock;

use regex::Regex;

use super::{StringStream, word};

static CHANGE_HEADER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r"^(Mon|Tue|Wed|Thu|Fri|Sat|Sun) ",
        r"(Jan|Feb|Mar|Apr|May|Jun|Jul|Aug|Sep|Oct|Nov|Dec)  ?",
        r"[0-9]{1,2} [0-9]{2}:[0-9]{2}(:[0-9]{2})? [A-Z]{3,4} [0-9]{4} - "
    ))
    .unwrap()
});
static EMAIL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[A-Za-z0-9_+.-]+@[A-Za-z0-9_.-]+").unwrap());

pub(super) fn changes_token(stream: &mut StringStream<'_>) -> Option<&'static str> {
    if stream.sol() {
        if stream.line().bytes().all(|ch| ch == b'-') && !stream.eol() {
            stream.skip_to_end();
            return Some("tag");
        }
        if stream.match_regex(&CHANGE_HEADER, true).is_some() {
            return Some("tag");
        }
    }
    if stream.match_regex(&EMAIL, true).is_some() {
        return Some("string");
    }
    stream.next();
    None
}

static PREAMBLE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[a-zA-Z0-9()]+:").unwrap());
static VARIABLE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\$([A-Za-z0-9_]+|\{[A-Za-z0-9_]+\})").unwrap());
static BRACED_MACRO: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^%\{\??[A-Za-z0-9_ :!\-]+\}").unwrap());

const SECTIONS: &[&str] = &[
    "%debug_package",
    "%package",
    "%description",
    "%prep",
    "%build",
    "%install",
    "%files",
    "%clean",
    "%changelog",
    "%preinstall",
    "%preun",
    "%postinstall",
    "%postun",
    "%pretrans",
    "%posttrans",
    "%pre",
    "%post",
    "%triggerin",
    "%triggerun",
    "%verifyscript",
    "%check",
    "%triggerpostun",
    "%triggerprein",
    "%trigger",
];
const ARCHES: &[&str] = &[
    "i386", "i586", "i686", "x86_64", "ppc64le", "ppc64", "ppc", "ia64", "s390x", "s390",
    "sparc64", "sparcv9", "sparc", "noarch", "alphaev6", "alpha", "hppa", "mipsel",
];
const OPERATORS: &[&str] = &["!", "?", "<=", "<", ">=", ">", "==", "&&", "||"];

#[derive(Clone, Debug, Default)]
pub(super) struct State {
    control_flow: bool,
    macro_parameters: bool,
}

impl State {
    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        if stream.peek() == Some('#') {
            stream.skip_to_end();
            return Some("comment");
        }
        if stream.sol() {
            if stream.match_regex(&PREAMBLE, true).is_some() {
                return Some("header");
            }
            if eat_prefix(stream, SECTIONS) {
                return Some("atom");
            }
        }
        if stream.match_regex(&VARIABLE, true).is_some() {
            return Some("def");
        }
        if eat_prefix(stream, &["%else", "%endif"]) {
            return Some("keyword");
        }
        if eat_prefix(stream, &["%ifnarch", "%ifarch", "%if"]) {
            self.control_flow = true;
            return Some("keyword");
        }
        if self.control_flow {
            if eat_prefix(stream, OPERATORS) {
                return Some("operator");
            }
            if stream.eat_while(|ch| ch.is_ascii_digit()) {
                return Some("number");
            }
            // An ordinary number, comment, or end of line does not reset the
            // flag in the tokenizer. The arch and braced macro paths can reset it.
            if stream.eol() {
                self.control_flow = false;
            }
        }
        if eat_prefix(stream, ARCHES) {
            self.control_flow &= !stream.eol();
            return Some("number");
        }
        let mut chars = stream.line()[stream.pos()..].chars();
        if chars.next() == Some('%') && chars.next().is_some_and(word) {
            stream.next();
            stream.eat_while(word);
            if stream.eat('(').is_some() {
                self.macro_parameters = true;
            }
            return Some("keyword");
        }
        if self.macro_parameters {
            if stream.eat_while(|ch| ch.is_ascii_digit()) {
                return Some("number");
            }
            if stream.eat(')').is_some() {
                self.macro_parameters = false;
                return Some("keyword");
            }
        }
        if stream.match_regex(&BRACED_MACRO, true).is_some() {
            self.control_flow &= !stream.eol();
            return Some("def");
        }
        stream.next();
        None
    }
}

fn eat_prefix(stream: &mut StringStream<'_>, prefixes: &[&str]) -> bool {
    // Preserve upstream alternation order. The regexes do not require a word boundary.
    prefixes
        .iter()
        .any(|prefix| stream.match_str(prefix, true, false))
}
