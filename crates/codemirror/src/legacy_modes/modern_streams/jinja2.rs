use std::sync::LazyLock;

use regex::Regex;

use super::{StringStream, in_words, remaining, take, word};

const KEYWORDS: &str = "and as block endblock by cycle debug else elif extends filter endfilter \
    firstof do for endfor if endif ifchanged endifchanged ifequal endifequal ifnotequal set raw \
    endraw endifnotequal in include load not now or parsed regroup reversed spaceless call \
    endcall macro endmacro endspaceless ssi templatetag openblock closeblock openvariable \
    closevariable without context openbrace closebrace opencomment closecomment widthratio url \
    with endwith get_current_language trans endtrans noop blocktrans endblocktrans \
    get_available_languages get_current_language_bidi pluralize autoescape endautoescape";
static NUMBER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(?:[0-9][+*/-])?[0-9]+(?:\.[0-9]+)?").unwrap());

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tag {
    Block,
    Expression,
    Line,
}

#[derive(Clone, Debug, Default)]
pub(super) struct State {
    comment: bool,
    tag: Option<Tag>,
    quote: Option<char>,
    parentheses: usize,
    brackets: usize,
    operator: bool,
    sign: bool,
}

impl State {
    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        let style = if self.comment {
            Some(self.comment(stream))
        } else if let Some(tag) = self.tag {
            Some(self.inside(stream, tag))
        } else {
            self.outside(stream)
        };
        if stream.eol()
            && self.tag == Some(Tag::Line)
            && self.quote.is_none()
            && self.parentheses == 0
            && self.brackets == 0
        {
            self.tag = None;
        }
        style
    }

    fn outside(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        if stream.eat('{').is_some() {
            if stream.eat('#').is_some() {
                return Some(self.comment(stream));
            }
            if let Some(ch) = stream.eat_if(|ch| matches!(ch, '{' | '%')) {
                self.open(if ch == '{' {
                    Tag::Expression
                } else {
                    Tag::Block
                });
                stream.eat('-');
                return Some("tag");
            }
        } else if stream.eat('#').is_some() {
            if stream.peek() == Some('#') {
                stream.skip_to_end();
                return Some("comment");
            }
            if !stream.eol() {
                self.open(Tag::Line);
                return Some("tag");
            }
        }
        stream.next();
        None
    }

    fn open(&mut self, tag: Tag) {
        self.tag = Some(tag);
        self.parentheses = 0;
        self.brackets = 0;
    }

    fn comment(&mut self, stream: &mut StringStream<'_>) -> &'static str {
        self.comment = !stream.skip_to("#}");
        if self.comment {
            stream.skip_to_end();
        } else {
            stream.eat_while(|ch| matches!(ch, '#' | '}'));
        }
        "comment"
    }

    fn inside(&mut self, stream: &mut StringStream<'_>, tag: Tag) -> &'static str {
        let ch = stream.peek();
        if std::mem::take(&mut self.operator)
            && let Some(style) = literal(stream)
        {
            return style;
        }
        if std::mem::take(&mut self.sign)
            && let Some(style) = literal(stream)
        {
            return style;
        }
        if let Some(quote) = self.quote {
            if ch == Some(quote) {
                self.quote = None;
            }
            stream.next();
            return "string";
        }
        match ch {
            Some('\'' | '"') => {
                self.quote = stream.next();
                return "string";
            }
            Some(')') if self.parentheses > 0 => {
                self.parentheses -= 1;
                stream.next();
            }
            Some('(') => {
                self.parentheses += 1;
                stream.next();
            }
            Some(']') if self.brackets > 0 => {
                self.brackets -= 1;
                stream.next();
            }
            Some('[') => {
                self.brackets += 1;
                stream.next();
            }
            _ => return self.inside_word(stream, tag),
        }
        "variable"
    }

    fn inside_word(&mut self, stream: &mut StringStream<'_>, tag: Tag) -> &'static str {
        let close = match tag {
            Tag::Block => Some("%}"),
            Tag::Expression => Some("}}"),
            Tag::Line => None,
        };
        // The hyphen is consumed even when it is not part of a closing tag.
        if close.is_some_and(|close| {
            take(stream, close) || stream.eat('-').is_some() && take(stream, close)
        }) {
            self.tag = None;
            return "tag";
        }
        if stream.eat_if(|ch| "+-*&%=<>!?|~^".contains(ch)).is_some() {
            self.operator = true;
            return "operator";
        }
        if stream
            .eat_if(|ch| matches!(ch, ':' | '[' | '(' | '{'))
            .is_some()
        {
            self.sign = true;
            return "variable";
        }
        if tag == Tag::Line && stream.column() == 1 && keyword(stream) {
            return "keyword";
        }
        if stream.eat(' ').is_some() || stream.sol() {
            if keyword(stream) {
                return "keyword";
            }
            if let Some(style) = literal(stream) {
                return style;
            }
            if stream.sol() {
                stream.next();
            }
        } else {
            stream.next();
        }
        "variable"
    }
}

fn keyword(stream: &mut StringStream<'_>) -> bool {
    let text = ascii_word(stream);
    in_words(KEYWORDS, text) && take(stream, text)
}

fn literal(stream: &mut StringStream<'_>) -> Option<&'static str> {
    let text = ascii_word(stream);
    if matches!(text, "true" | "false") {
        take(stream, text);
        Some("atom")
    } else if stream.match_regex(&NUMBER, true).is_some() {
        Some("number")
    } else {
        None
    }
}

fn ascii_word<'line>(stream: &StringStream<'line>) -> &'line str {
    let text = remaining(stream);
    &text[..text.find(|ch| !word(ch)).unwrap_or(text.len())]
}
