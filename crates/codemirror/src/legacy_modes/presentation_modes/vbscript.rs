use std::{collections::HashSet, sync::LazyLock};

use super::{advance, remaining, starts_ascii_case, word, word_prefix};
use crate::{legacy_modes::js_space, stream_parser::StringStream};

#[derive(Clone, Debug, Default)]
pub(super) struct State {
    indent: isize,
    skip_indent: bool,
    dotted_string: bool,
}

impl State {
    pub(super) fn token(
        &mut self,
        asp: bool,
        stream: &mut StringStream<'_>,
    ) -> Option<&'static str> {
        if stream.sol() {
            self.skip_indent = false;
        }
        let style = self.base(asp, stream);
        if stream.current() != "." {
            return style;
        }
        match self.base(asp, stream) {
            Some(style @ ("variable" | "variableName.special" | "builtin" | "keyword")) => {
                if known(&stream.current()[1..], asp) {
                    Some("keyword")
                } else if matches!(style, "builtin" | "keyword") {
                    Some("variable")
                } else {
                    Some(style)
                }
            }
            _ => Some("error"),
        }
    }

    fn base(&mut self, asp: bool, stream: &mut StringStream<'_>) -> Option<&'static str> {
        if self.dotted_string {
            stream.skip_to_end();
            return Some("string");
        }
        if stream.eat_space() {
            return None;
        }
        let rest = remaining(stream);
        if rest.starts_with('\'') || word_prefix(rest, "rem") {
            stream.skip_to_end();
            return Some("comment");
        }
        if let Some(len) = number(rest) {
            advance(stream, len);
            return Some("number");
        }
        if stream.eat('"').is_some() {
            // The delimiter is the entire current token. After a dot it is .",
            // which cannot match where the upstream string scanner looks for it.
            self.dotted_string = stream.current() == ".\"";
            if self.dotted_string || !stream.skip_to("\"") {
                stream.skip_to_end();
            } else {
                stream.next();
            }
            return Some("string");
        }
        if ["<>", "<=", ">="]
            .iter()
            .any(|op| stream.match_str(op, true, false))
            || stream.eat_if(|ch| "+-*/&\\^<>=".contains(ch)).is_some()
        {
            return Some("operator");
        }
        match stream.next() {
            Some('.' | ',') => None,
            Some('(' | ')') => Some("bracket"),
            Some(ch) if ch.is_ascii_alphabetic() => Some(self.identifier(asp, rest, stream)),
            _ => Some("error"),
        }
    }

    fn identifier(&mut self, asp: bool, rest: &str, stream: &mut StringStream<'_>) -> &'static str {
        if let Some(phrase) = PHRASES.iter().find(|&&phrase| word_prefix(rest, phrase)) {
            // The caller has already consumed the first character.
            advance(stream, phrase.len() - 1);
            if *phrase == "on error resume next" {
                self.skip_indent = true;
            }
            return "keyword";
        }
        stream.eat_while(word);
        let name = rest[..rest.len() - remaining(stream).len()].to_ascii_lowercase();
        let name = name.as_str();
        if matches!(
            name,
            "and" | "or" | "not" | "xor" | "is" | "mod" | "eqv" | "imp"
        ) {
            "operator"
        } else if name == "exit" {
            self.skip_indent = true;
            "keyword"
        } else if name == "do" {
            self.indent = self.indent.saturating_add(1);
            self.skip_indent = true;
            "keyword"
        } else if OPENING.contains(&name) || CLOSING.contains(&name) {
            if std::mem::take(&mut self.skip_indent) {
                return "keyword";
            }
            if OPENING.contains(&name) {
                self.indent = self.indent.saturating_add(1);
            } else {
                self.indent = self.indent.saturating_sub(1);
            }
            "keyword"
        } else if name == "end" {
            self.indent = self.indent.saturating_sub(2);
            "keyword"
        } else if MIDDLE.contains(&name) || KEYWORDS.contains(&name) {
            "keyword"
        } else if matches!(name, "true" | "false" | "nothing" | "empty" | "null") {
            "atom"
        } else if known(name, asp) {
            "variableName.special"
        } else if BUILTINS.contains(name) || asp && ASP_BUILTINS.contains(&name) {
            "builtin"
        } else {
            "variable"
        }
    }

    pub(super) fn indent(&self, text_after: &str, unit: usize) -> usize {
        let text = text_after.trim_matches(js_space);
        let outdent = CLOSING
            .iter()
            .chain(MIDDLE)
            .chain([&"end"])
            .any(|keyword| word_prefix(text, keyword));
        let steps = self.indent.saturating_sub(isize::from(outdent));
        usize::try_from(steps).unwrap_or(0).saturating_mul(unit)
    }
}

fn number(text: &str) -> Option<usize> {
    let prefix = ["&h", "&o"]
        .iter()
        .position(|prefix| starts_ascii_case(text, prefix));
    let digits = text.get(prefix.map_or(0, |_| 2)..)?;
    let tail = digits.trim_start_matches(|ch: char| ch.is_ascii_digit() || ch == '.');
    if digits.len() == tail.len()
        || tail.starts_with(|ch: char| ch.is_ascii_alphabetic() || ch == '_')
    {
        return None;
    }

    let decimal_tail = text.trim_start_matches(|ch: char| ch.is_ascii_digit());
    if let Some(after_dot) = decimal_tail.strip_prefix('.') {
        let after_fraction = after_dot.trim_start_matches(|ch: char| ch.is_ascii_digit());
        if decimal_tail.len() != text.len() || after_dot.len() != after_fraction.len() {
            return Some(text.len() - eat_suffix(after_fraction, 'j').len());
        }
    }
    let tail = match prefix {
        Some(0) => digits.trim_start_matches(|ch: char| ch.is_ascii_hexdigit()),
        Some(_) => digits.trim_start_matches(|ch| matches!(ch, '0'..='7')),
        None if text.starts_with(|ch| matches!(ch, '1'..='9')) => {
            eat_suffix(decimal_tail.strip_prefix('F').unwrap_or(decimal_tail), 'j')
        }
        None => text.strip_prefix('0').filter(|tail| {
            !tail.starts_with(|ch: char| ch.is_ascii_digit() || ch.eq_ignore_ascii_case(&'x'))
        })?,
    };
    if prefix.is_some() && digits.len() == tail.len() {
        return None;
    }
    Some(text.len() - eat_suffix(tail, 'l').len())
}

fn eat_suffix(text: &str, suffix: char) -> &str {
    text.strip_prefix([suffix, suffix.to_ascii_uppercase()])
        .unwrap_or(text)
}

fn known(name: &str, asp: bool) -> bool {
    KNOWN.contains(name) || asp && ASP_KNOWN.contains(name)
}

const OPENING: &[&str] = &[
    "class", "sub", "select", "while", "if", "function", "property", "with", "for",
];
const MIDDLE: &[&str] = &["else", "elseif", "case"];
const CLOSING: &[&str] = &["next", "loop", "wend"];
const PHRASES: &[&str] = &["on error resume next", "on error goto 0", "option explicit"];
const KEYWORDS: &[&str] = &[
    "dim",
    "redim",
    "then",
    "until",
    "randomize",
    "byval",
    "byref",
    "new",
    "in",
    "const",
    "private",
    "public",
    "get",
    "set",
    "let",
    "stop",
    "call",
    "me",
];
const ASP_BUILTINS: &[&str] = &["server", "response", "request", "session", "application"];
static KNOWN: LazyLock<HashSet<&str>> = LazyLock::new(|| {
    "clear execute raise replace test write writeline close open state eof update addnew end \
     createobject quit description firstindex global helpcontext helpfile ignorecase length number \
     pattern source value count"
        .split_ascii_whitespace()
        .collect()
});
static ASP_KNOWN: LazyLock<HashSet<&str>> = LazyLock::new(|| {
    "addheader appendtolog binarywrite end flush redirect binaryread remove removeall lock unlock \
     abandon getlasterror htmlencode mappath transfer urlencode buffer cachecontrol charset \
     contenttype expires expiresabsolute isclientconnected pics status clientcertificate cookies \
     form querystring servervariables totalbytes contents staticobjects codepage lcid sessionid \
     timeout scripttimeout"
        .split_ascii_whitespace()
        .collect()
});
static BUILTINS: LazyLock<HashSet<&str>> = LazyLock::new(|| {
    "abs array asc atn cbool cbyte ccur cdate cdbl chr cint clng cos csng cstr date dateadd \
     datediff datepart dateserial datevalue day escape eval execute exp filter formatcurrency \
     formatdatetime formatnumber formatpercent getlocale getobject getref hex hour inputbox instr \
     instrrev int fix isarray isdate isempty isnull isnumeric isobject join lbound lcase left len \
     loadpicture log ltrim rtrim trim maths mid minute month monthname msgbox now oct replace rgb \
     right rnd round scriptengine scriptenginebuildversion scriptenginemajorversion \
     scriptengineminorversion second setlocale sgn sin space split sqr strcomp string strreverse \
     tan time timer timeserial timevalue typename ubound ucase unescape vartype weekday weekdayname \
     year wscript err debug regexp vbblack vbred vbgreen vbyellow vbblue vbmagenta vbcyan vbwhite \
     vbbinarycompare vbtextcompare vbsunday vbmonday vbtuesday vbwednesday vbthursday vbfriday \
     vbsaturday vbusesystemdayofweek vbfirstjan1 vbfirstfourdays vbfirstfullweek vbgeneraldate \
     vblongdate vbshortdate vblongtime vbshorttime vbobjecterror vbokonly vbokcancel \
     vbabortretryignore vbyesnocancel vbyesno vbretrycancel vbcritical vbquestion vbexclamation \
     vbinformation vbdefaultbutton1 vbdefaultbutton2 vbdefaultbutton3 vbdefaultbutton4 \
     vbapplicationmodal vbsystemmodal vbok vbcancel vbabort vbretry vbignore vbyes vbno vbcr vbcrlf \
     vbformfeed vblf vbnewline vbnullchar vbnullstring vbtab vbverticaltab vbusedefault vbtrue \
     vbfalse vbempty vbnull vbinteger vblong vbsingle vbdouble vbcurrency vbdate vbstring vbobject \
     vberror vbboolean vbvariant vbdataobject vbdecimal vbbyte vbarray"
        .split_ascii_whitespace()
        .collect()
});
