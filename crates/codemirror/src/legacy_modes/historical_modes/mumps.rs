use std::sync::LazyLock;

use regex::Regex;

use super::{StringStream, leading_word, word};

const COMMANDS: &str = "break close do else for goto halt hang if job kill lock merge new open quit read set tcommit trollback tstart use view write xecute b c d e f g h i j k l m n o q r s tc tro ts u v w x";
const INTRINSICS: &str = "ascii char data ecode estack etrap extract find fnumber get horolog io increment job justify length name next order piece qlength qsubscript query quit random reverse select stack test text translate view x y a c d e ec es et f fn g h i j l n na o p q ql qs r re s st t tr v z";
static NUMBER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[-+]?[0-9]+(?:\.[0-9]+)?(?:[eE][-+]?[0-9]+)?").unwrap());

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Context {
    #[default]
    Argument,
    Command,
    CommandToken,
    Postcondition,
}

#[derive(Clone, Debug, Default)]
pub(super) struct State {
    label: bool,
    context: Context,
}

impl State {
    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        if stream.sol() {
            self.label = true;
            self.context = Context::Argument;
        }
        let ch = stream.peek()?;
        if matches!(ch, ' ' | '\t') {
            self.label = false;
            self.context = match self.context {
                Context::Argument | Context::Command => Context::Command,
                Context::CommandToken | Context::Postcondition => Context::Argument,
            };
        } else if ch != '.' && self.in_command() {
            self.context = if ch == ':' {
                Context::Postcondition
            } else {
                Context::CommandToken
            };
        }
        if ch == '(' {
            self.label = false;
        }
        let style = self.base(stream, ch);
        if self.label { Some("tag") } else { style }
    }

    fn in_command(&self) -> bool {
        matches!(self.context, Context::Command | Context::CommandToken)
    }

    fn base(&self, stream: &mut StringStream<'_>, ch: char) -> Option<&'static str> {
        if ch == ';' {
            stream.skip_to_end();
            Some("comment")
        } else if stream.match_regex(&NUMBER, true).is_some() {
            Some("number")
        } else if ch == '"' {
            // Upstream looks for a quote from the current cursor, including
            // the opening quote. Thus it consumes each quote by itself.
            stream.next();
            Some("string")
        } else if ["'=", "<=", ">=", "'>", "'<", "[]", ")]", "|]", "(]"]
            .iter()
            .any(|pair| stream.match_str(pair, true, false))
        {
            // The upstream square brackets in the double-operator regex
            // form a character class. It does not group [[ or ]].
            Some("operator")
        } else if "+-*/&#!_?\\<>='[]".contains(ch) {
            stream.next();
            Some("operator")
        } else if matches!(ch, '.' | ',' | ':') {
            stream.next();
            None
        } else if matches!(ch, '(' | ')') {
            stream.next();
            Some("bracket")
        } else if self.in_command() && match_word(stream, 0, COMMANDS) {
            Some("controlKeyword")
        } else if ch == '$' && match_word(stream, 1, INTRINSICS) {
            Some("builtin")
        } else if ch == '%' || ch.is_ascii_alphabetic() {
            stream.next();
            stream.eat_while(|ch| ch.is_ascii_alphanumeric());
            Some("variable")
        } else if matches!(ch, '$' | '^') {
            stream.next();
            Some("builtin")
        } else if ch == '@' {
            stream.next();
            Some("string.special")
        } else if word(ch) {
            stream.eat_while(|ch| word(ch) || ch == '%');
            Some("variable")
        } else {
            stream.next();
            Some("error")
        }
    }
}

fn match_word(stream: &mut StringStream<'_>, prefix_len: usize, words: &str) -> bool {
    let from = stream.pos();
    let name = leading_word(&stream.line()[from + prefix_len..]);
    if words
        .split_ascii_whitespace()
        .any(|word| word.eq_ignore_ascii_case(name))
    {
        stream.set_pos(from + prefix_len + name.len()).unwrap();
        true
    } else {
        false
    }
}
