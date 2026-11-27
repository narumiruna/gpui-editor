use super::{LegacyMode, js_space, xml_factory::LegacyXmlConfig};
use crate::stream_parser::StringStream;

#[derive(Clone, Debug)]
pub(super) struct State {
    pub(super) mode: LegacyMode,
    scanner: Scanner,
}

impl State {
    pub(super) fn new(mode: LegacyMode) -> Self {
        Self {
            mode,
            scanner: Scanner::default(),
        }
    }

    fn config(&self) -> &'static LegacyXmlConfig {
        if self.mode == LegacyMode::Html {
            &LegacyXmlConfig::HTML
        } else {
            &LegacyXmlConfig::XML
        }
    }

    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        self.scanner.token(self.config(), stream)
    }

    pub(super) fn indent(&self, text: &str, unit: usize) -> Option<usize> {
        self.scanner.indent(self.config(), text, unit)
    }
}

#[derive(Clone, Debug, Default)]
pub(super) struct Scanner {
    scan: Scan,
    parse: Parse,
    indented: usize,
    tag_name: Option<String>,
    tag_start: Option<usize>,
    context: Vec<Context>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum Scan {
    #[default]
    Text,
    Tag,
    Attribute(char, usize),
    Block(&'static str, &'static str),
    Doctype(usize),
}

#[derive(Clone, Copy, Debug, Default)]
enum Parse {
    #[default]
    Base,
    OpenName,
    CloseName,
    Close(bool),
    Attributes,
    AttrEq,
    AttrValue,
    AttrContinued,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Token {
    OpenTag,
    CloseTag,
    EndTag,
    SelfCloseTag,
    Equals,
    Word,
    String,
    Other(Option<&'static str>),
}

impl Token {
    fn style(self) -> Option<&'static str> {
        match self {
            Self::OpenTag | Self::CloseTag | Self::EndTag | Self::SelfCloseTag => {
                Some("angleBracket")
            }
            Self::Word => Some("word"),
            Self::String => Some("string"),
            Self::Equals => None,
            Self::Other(style) => style,
        }
    }
}

#[derive(Clone, Debug)]
struct Context {
    tag: String,
    indent: usize,
    start_of_line: bool,
    no_indent: bool,
}

impl Scanner {
    pub(super) fn skip_attribute(&mut self) {
        if matches!(self.parse, Parse::AttrValue) {
            self.parse = Parse::Attributes;
        }
    }

    pub(super) fn current_tag(&self) -> Option<&str> {
        self.tag_name.as_deref()
    }

    pub(super) fn current_context(&self) -> impl Iterator<Item = &str> {
        self.context.iter().map(|cx| cx.tag.as_str())
    }

    pub(super) fn token(
        &mut self,
        config: &LegacyXmlConfig,
        stream: &mut StringStream<'_>,
    ) -> Option<&'static str> {
        if self.tag_name.is_none() && stream.sol() {
            self.indented = stream.indentation();
        }
        if stream.eat_space() {
            return None;
        }
        let token = match self.scan {
            Scan::Text => self.text(stream),
            Scan::Tag => self.tag(stream),
            Scan::Attribute(quote, _) => self.attribute(stream, quote),
            Scan::Block(style, end) => self.block(stream, style, end),
            Scan::Doctype(depth) => self.doctype(stream, depth),
        };
        match token {
            Token::Other(None | Some("comment" | "invalid")) => token.style(),
            _ => self.parse(config, token, stream),
        }
    }

    fn text(&mut self, stream: &mut StringStream<'_>) -> Token {
        match stream.next() {
            Some('<') if stream.eat('!').is_some() => {
                if stream.eat('[').is_some() {
                    if stream.match_str("CDATA[", true, false) {
                        return self.block(stream, "atom", "]]>");
                    }
                } else if stream.match_str("--", true, false) {
                    return self.block(stream, "comment", "-->");
                } else if stream.match_str("DOCTYPE", true, true) {
                    stream.eat_while(name_char);
                    return self.doctype(stream, 1);
                }
                Token::Other(None)
            }
            Some('<') if stream.eat('?').is_some() => {
                stream.eat_while(name_char);
                self.scan = Scan::Block("meta", "?>");
                Token::Other(Some("meta"))
            }
            Some('<') => {
                self.scan = Scan::Tag;
                if stream.eat('/').is_some() {
                    Token::CloseTag
                } else {
                    Token::OpenTag
                }
            }
            Some('&') => {
                let digits = if stream.eat('#').is_some() {
                    if stream.eat('x').is_some() {
                        stream.eat_while(|ch| ch.is_ascii_hexdigit())
                    } else {
                        stream.eat_while(|ch| ch.is_ascii_digit())
                    }
                } else {
                    stream.eat_while(|ch| name_char(ch) || ch == ':')
                };
                Token::Other(Some(if digits && stream.eat(';').is_some() {
                    "atom"
                } else {
                    "error"
                }))
            }
            _ => {
                stream.eat_while(|ch| !matches!(ch, '&' | '<'));
                Token::Other(None)
            }
        }
    }

    fn tag(&mut self, stream: &mut StringStream<'_>) -> Token {
        match stream.next() {
            Some('>') => {
                self.scan = Scan::Text;
                Token::EndTag
            }
            Some('/') if stream.eat('>').is_some() => {
                self.scan = Scan::Text;
                Token::SelfCloseTag
            }
            Some('=') => Token::Equals,
            Some('<') => {
                self.scan = Scan::Text;
                self.parse = Parse::Base;
                self.tag_name = None;
                self.tag_start = None;
                // The upstream text scanner continues *after* the second '<'.
                match self.text(stream) {
                    Token::OpenTag => {
                        self.tag_start = Some(stream.column());
                        self.parse = Parse::OpenName;
                    }
                    Token::CloseTag => self.parse = Parse::CloseName,
                    _ => {}
                }
                Token::Other(Some("invalid"))
            }
            Some(quote @ ('\'' | '"')) => {
                self.scan = Scan::Attribute(quote, stream.column());
                self.attribute(stream, quote)
            }
            _ => {
                let rest = &stream.line()[stream.pos()..];
                let end = rest
                    .find(|ch| js_space(ch) || matches!(ch, '=' | '<' | '>' | '"' | '\''))
                    .unwrap_or(rest.len());
                let bytes = rest[..end].trim_end_matches('/').len();
                stream.set_pos(stream.pos() + bytes).unwrap();
                Token::Word
            }
        }
    }

    fn attribute(&mut self, stream: &mut StringStream<'_>, quote: char) -> Token {
        if stream.any(|ch| ch == quote) {
            self.scan = Scan::Tag;
        }
        Token::String
    }

    fn block(
        &mut self,
        stream: &mut StringStream<'_>,
        style: &'static str,
        end: &'static str,
    ) -> Token {
        if stream.skip_to(end) {
            stream.match_str(end, true, false);
            self.scan = Scan::Text;
        } else {
            stream.skip_to_end();
            self.scan = Scan::Block(style, end);
        }
        Token::Other(Some(style))
    }

    fn doctype(&mut self, stream: &mut StringStream<'_>, mut depth: usize) -> Token {
        for ch in stream {
            match ch {
                '<' => depth += 1,
                '>' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
        }
        self.scan = if depth == 0 {
            Scan::Text
        } else {
            Scan::Doctype(depth)
        };
        Token::Other(Some("meta"))
    }

    fn parse(
        &mut self,
        config: &LegacyXmlConfig,
        token: Token,
        stream: &StringStream<'_>,
    ) -> Option<&'static str> {
        let mut style = token.style();
        loop {
            self.parse = match self.parse {
                Parse::Base => match token {
                    Token::OpenTag => {
                        self.tag_start = Some(stream.column());
                        Parse::OpenName
                    }
                    Token::CloseTag => Parse::CloseName,
                    _ => Parse::Base,
                },
                Parse::OpenName if token == Token::Word => {
                    self.tag_name = Some(stream.current().to_owned());
                    style = Some("tag");
                    Parse::Attributes
                }
                Parse::OpenName if config.allow_missing_tag_name && token == Token::EndTag => {
                    self.parse = Parse::Attributes;
                    continue;
                }
                Parse::OpenName => {
                    style = Some("error");
                    Parse::OpenName
                }
                Parse::CloseName if config.allow_missing_tag_name && token == Token::EndTag => {
                    self.parse = Parse::Close(true);
                    continue;
                }
                Parse::CloseName => {
                    let valid = if token == Token::Word {
                        let tag = stream.current();
                        if self
                            .context
                            .last()
                            .is_some_and(|cx| cx.tag != tag && config.implicit(&cx.tag))
                        {
                            self.context.pop();
                        }
                        !config.match_closing || self.context.last().is_some_and(|cx| cx.tag == tag)
                    } else {
                        false
                    };
                    style = Some(if valid { "tag" } else { "error" });
                    Parse::Close(valid)
                }
                Parse::Close(valid) => {
                    if !valid || token != Token::EndTag {
                        style = Some("error");
                    }
                    if token == Token::EndTag {
                        self.context.pop();
                        Parse::Base
                    } else {
                        // Error mode delegates to closeState; an extra token can clear it.
                        Parse::Close(true)
                    }
                }
                Parse::Attributes => match token {
                    Token::Word => {
                        style = Some("attribute");
                        Parse::AttrEq
                    }
                    Token::EndTag | Token::SelfCloseTag => {
                        let tag = self.tag_name.take();
                        let tag_start = self.tag_start.take();
                        // JavaScript uses the key "null" for an unnamed opening tag.
                        let key = tag.as_deref().unwrap_or("null");
                        while self
                            .context
                            .last()
                            .is_some_and(|cx| config.grabs(&cx.tag, key))
                        {
                            self.context.pop();
                        }
                        if token != Token::SelfCloseTag && !config.auto_closes(key) {
                            self.context.push(Context {
                                no_indent: config.no_indent(key)
                                    || self.context.last().is_some_and(|cx| cx.no_indent),
                                tag: tag.unwrap_or_default(),
                                indent: self.indented,
                                start_of_line: tag_start == Some(self.indented),
                            });
                        }
                        Parse::Base
                    }
                    _ => {
                        style = Some("error");
                        Parse::Attributes
                    }
                },
                Parse::AttrEq if token == Token::Equals => Parse::AttrValue,
                Parse::AttrValue if token == Token::String => Parse::AttrContinued,
                Parse::AttrValue if token == Token::Word && config.allows_unquoted() => {
                    style = Some("string");
                    Parse::Attributes
                }
                Parse::AttrContinued if token == Token::String => Parse::AttrContinued,
                Parse::AttrEq | Parse::AttrValue | Parse::AttrContinued => {
                    if matches!(self.parse, Parse::AttrValue)
                        || (matches!(self.parse, Parse::AttrEq) && !config.allows_missing())
                    {
                        style = Some("error");
                    }
                    self.parse = Parse::Attributes;
                    continue;
                }
            };
            return style;
        }
    }

    pub(super) fn indent(
        &self,
        config: &LegacyXmlConfig,
        text: &str,
        unit: usize,
    ) -> Option<usize> {
        if let Scan::Attribute(_, column) = self.scan {
            return Some(if self.tag_start == Some(self.indented) {
                column.saturating_add(1)
            } else {
                self.indented.saturating_add(unit)
            });
        }
        let mut stack = self.context.as_slice();
        if stack.last().is_some_and(|cx| cx.no_indent)
            || !matches!(self.scan, Scan::Tag | Scan::Text)
        {
            return None;
        }
        if let (Some(start), Some(tag)) = (self.tag_start, &self.tag_name) {
            let offset = if config.multiline_tag_indent_past_tag {
                tag.chars().count() + 2
            } else {
                unit.saturating_mul(config.multiline_tag_indent_factor.max(1))
            };
            return Some(start.saturating_add(offset));
        }
        if config.align_cdata && text.contains("<![CDATA[") {
            return Some(0);
        }
        if let Some(tail) = text.strip_prefix('<') {
            let closing = tail.starts_with('/');
            let tail = tail.strip_prefix('/').unwrap_or(tail);
            let end = tail
                .find(|ch| !(name_char(ch) || ch == ':'))
                .unwrap_or(tail.len());
            let tag = &tail[..end];
            while let Some((context, prev)) = stack.split_last() {
                if closing && context.tag == tag {
                    stack = prev;
                    break;
                }
                if if closing {
                    config.implicit(&context.tag)
                } else {
                    config.grabs(&context.tag, tag)
                } {
                    stack = prev;
                } else {
                    break;
                }
            }
        }
        while stack.len() > 1 && stack.last().is_some_and(|cx| !cx.start_of_line) {
            stack = &stack[..stack.len() - 1];
        }
        Some(stack.last().map_or(0, |cx| cx.indent.saturating_add(unit)))
    }
}

pub(super) fn word(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

fn name_char(ch: char) -> bool {
    word(ch) || matches!(ch, '.' | '-')
}

pub(super) fn html_auto_closes(tag: &str) -> bool {
    matches!(
        tag,
        "area"
            | "base"
            | "br"
            | "col"
            | "command"
            | "embed"
            | "frame"
            | "hr"
            | "img"
            | "input"
            | "keygen"
            | "link"
            | "meta"
            | "param"
            | "source"
            | "track"
            | "wbr"
            | "menuitem"
    )
}

pub(super) fn html_implicit(tag: &str) -> bool {
    matches!(
        tag,
        "dd" | "li"
            | "optgroup"
            | "option"
            | "p"
            | "rp"
            | "rt"
            | "tbody"
            | "td"
            | "tfoot"
            | "th"
            | "tr"
    )
}

pub(super) fn html_grabs(parent: &str, next: &str) -> bool {
    match parent {
        "dd" | "dt" => matches!(next, "dd" | "dt"),
        "li" => next == "li",
        "option" => matches!(next, "option" | "optgroup"),
        "optgroup" => next == "optgroup",
        "p" => matches!(
            next,
            "address"
                | "article"
                | "aside"
                | "blockquote"
                | "dir"
                | "div"
                | "dl"
                | "fieldset"
                | "footer"
                | "form"
                | "h1"
                | "h2"
                | "h3"
                | "h4"
                | "h5"
                | "h6"
                | "header"
                | "hgroup"
                | "hr"
                | "menu"
                | "nav"
                | "ol"
                | "p"
                | "pre"
                | "section"
                | "table"
                | "ul"
        ),
        "rp" | "rt" => matches!(next, "rp" | "rt"),
        "tbody" | "thead" => matches!(next, "tbody" | "tfoot"),
        "td" | "th" => matches!(next, "td" | "th"),
        "tfoot" => next == "tbody",
        "tr" => next == "tr",
        _ => false,
    }
}
