use std::sync::LazyLock;

use regex::Regex;

use super::{StringStream, listed, word};

const KEYWORDS: &str =
    "class define site node include import inherits case if else in and elsif default or";
const ATOMS: &str = "false true running present absent file directory undef";
const BUILTINS: &str = "action augeas burst chain computer cron destination dport exec file filebucket group host icmp iniface interface jump k5login limit log_level log_prefix macauthorization mailalias maillist mcx mount nagios_command nagios_contact nagios_contactgroup nagios_host nagios_hostdependency nagios_hostescalation nagios_hostextinfo nagios_hostgroup nagios_service nagios_servicedependency nagios_serviceescalation nagios_serviceextinfo nagios_servicegroup nagios_timeperiod name notify outiface package proto reject resources router schedule scheduled_task selboolean selmodule service source sport ssh_authorized_key sshkey stage state table tidy todest toports tosource user vlan yumrepo zfs zone zpool";
const SPACE: &str =
    r"[\t-\r \u{a0}\u{1680}\u{2000}-\u{200a}\u{2028}\u{2029}\u{202f}\u{205f}\u{3000}\u{feff}]";

static VARIABLE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\{?(?:[a-z][a-z0-9_]*)?(?:(?:::[a-z][a-z0-9_]*)*::)?[a-zA-Z0-9_]+\}?").unwrap()
});
static ATTRIBUTE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!(r"^[a-zA-Z0-9_]+{SPACE}+=>")).unwrap());
static RESOURCE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!(r"^(@{{1,2}})?[a-zA-Z0-9_:]+{SPACE}*\{{")).unwrap());
static DEFINITION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!(r"^{SPACE}*[a-zA-Z0-9_:]+{SPACE}*")).unwrap());
static RESOURCE_NAME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!(r"^{SPACE}*[a-zA-Z0-9_:]+")).unwrap());
static OPEN_BRACE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!(r"^{SPACE}+\{{")).unwrap());
static INCLUDE: LazyLock<Regex> = LazyLock::new(|| {
    let non_space = SPACE.replacen('[', "[^", 1);
    Regex::new(&format!(r"^{SPACE}*{non_space}+{SPACE}*")).unwrap()
});
static FUNCTION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!(r"^{SPACE}*[a-zA-Z0-9_]+\(")).unwrap());
static DEFINITION_START: LazyLock<Regex> = LazyLock::new(|| {
    let non_space = SPACE.replacen('[', "[^", 1);
    Regex::new(&format!(r"^{SPACE}+{non_space}+{SPACE}+\{{")).unwrap()
});

#[derive(Clone, Debug, Default)]
pub(super) struct State {
    in_definition: bool,
    in_include: bool,
    quote: Option<char>,
}

impl State {
    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        if stream.eat_space() {
            return None;
        }
        let remaining = &stream.line()[stream.pos()..];
        let ch = stream.next()?;
        if ch == '$' {
            return Some(if stream.match_regex(&VARIABLE, true).is_none() {
                "error"
            } else if self.quote.is_some() {
                "variableName.special"
            } else {
                "variable"
            });
        }
        if let Some(quote) = self.quote {
            stream.back_up(ch.len_utf8()).unwrap();
            return Some(self.string(stream, quote));
        }
        if self.in_definition {
            if stream.match_regex(&DEFINITION, true).is_some() {
                return Some("def");
            }
            stream.match_regex(&OPEN_BRACE, true);
            self.in_definition = false;
        }
        if self.in_include {
            stream.match_regex(&INCLUDE, true);
            self.in_include = false;
            return Some("def");
        }
        if stream.match_regex(&FUNCTION, true).is_some() {
            stream.back_up(1).unwrap();
            return Some("def");
        }
        if ATTRIBUTE.is_match(remaining) {
            stream.eat_while(word);
            return Some("tag");
        }
        let initial_word = &remaining[..remaining
            .bytes()
            .take_while(|&b| word(char::from(b)))
            .count()];
        if let Some(style) = word_style(initial_word) {
            rewind_word(stream);
            self.in_definition = stream.match_regex(&DEFINITION_START, false).is_some();
            if initial_word == "include" {
                self.in_include = true;
            }
            return Some(style);
        }
        if initial_word.len() > 1 && ch.is_ascii_uppercase() {
            back_up_scalar(stream);
            stream.eat_while(resource_part);
            return Some("def");
        }
        if let Some(resource) = RESOURCE.captures(remaining) {
            if resource.get(1).is_some() {
                stream.eat('@');
                return Some("atom");
            }
            stream.match_regex(&RESOURCE_NAME, true);
            return Some("def");
        }
        match ch {
            '#' => {
                stream.skip_to_end();
                Some("comment")
            }
            '\'' | '"' => Some(self.string(stream, ch)),
            '{' | '}' => Some("bracket"),
            '/' => {
                if stream.skip_to("/") {
                    stream.eat('/');
                }
                Some("string.special")
            }
            '0'..='9' => {
                stream.eat_while(|ch| ch.is_ascii_digit());
                Some("number")
            }
            '=' => {
                stream.eat('>');
                Some("operator")
            }
            _ => {
                stream.eat_while(|ch| word(ch) || ch == '-');
                None
            }
        }
    }

    fn string(&mut self, stream: &mut StringStream<'_>, quote: char) -> &'static str {
        let mut escaped = false;
        self.quote = Some(quote);
        for ch in &mut *stream {
            if ch == quote {
                self.quote = None;
                break;
            }
            if ch == '$' && !escaped && quote == '"' {
                stream.back_up(1).unwrap();
                break;
            }
            escaped = ch == '\\';
        }
        "string"
    }
}

fn word_style(text: &str) -> Option<&'static str> {
    if listed(BUILTINS, text) {
        Some("builtin")
    } else if listed(ATOMS, text) {
        Some("atom")
    } else if listed(KEYWORDS, text) {
        Some("keyword")
    } else {
        None
    }
}

fn resource_part(ch: char) -> bool {
    word(ch) || ch == ':'
}

fn rewind_word(stream: &mut StringStream<'_>) {
    back_up_scalar(stream);
    stream.eat_while(word);
}

fn back_up_scalar(stream: &mut StringStream<'_>) {
    let width = stream.line()[..stream.pos()]
        .chars()
        .next_back()
        .unwrap()
        .len_utf8();
    stream.back_up(width).unwrap();
}
