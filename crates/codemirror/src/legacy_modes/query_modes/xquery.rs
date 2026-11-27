use super::{js_space, word};
use crate::stream_parser::StringStream;

const KEYWORDS: &str = "after all allowing ancestor ancestor-or-self any array as ascending at attribute base-uri before boundary-space by case cast castable catch child collation comment construction contains content context copy copy-namespaces count decimal-format declare default delete descendant descendant-or-self descending diacritics different distance document document-node element else empty empty-sequence encoding end entire every exactly except external first following following-sibling for from ftand ftnot ft-option ftor function fuzzy greatest group if import in inherit insensitive insert instance intersect into invoke is item language last lax least let levels lowercase map modify module most namespace next no node nodes no-inherit no-preserve not occurs of only option order ordered ordering paragraph paragraphs parent phrase preceding preceding-sibling preserve previous processing-instruction relationship rename replace return revalidation same satisfies schema schema-attribute schema-element score self sensitive sentence sentences sequence skip sliding some stable start stemming stop strict strip switch text then thesaurus times to transform treat try tumbling type typeswitch union unordered update updating uppercase using validate value variable version weight when where wildcards window with without word words xquery";
const TYPES: &str = "xs:anyAtomicType xs:anySimpleType xs:anyType xs:anyURI xs:base64Binary xs:boolean xs:byte xs:date xs:dateTime xs:dateTimeStamp xs:dayTimeDuration xs:decimal xs:double xs:duration xs:ENTITIES xs:ENTITY xs:float xs:gDay xs:gMonth xs:gMonthDay xs:gYear xs:gYearMonth xs:hexBinary xs:ID xs:IDREF xs:IDREFS xs:int xs:integer xs:item xs:java xs:language xs:long xs:Name xs:NCName xs:negativeInteger xs:NMTOKEN xs:NMTOKENS xs:nonNegativeInteger xs:nonPositiveInteger xs:normalizedString xs:NOTATION xs:numeric xs:positiveInteger xs:precisionDecimal xs:QName xs:short xs:string xs:time xs:token xs:unsignedByte xs:unsignedInt xs:unsignedLong xs:unsignedShort xs:untyped xs:untypedAtomic xs:yearMonthDuration";
const OPERATORS: &str = "eq ne lt le gt ge := = > >= < <= . | ? and or div idiv mod * / + -";
const AXES: &str = "self:: attribute:: child:: descendant:: descendant-or-self:: parent:: ancestor:: ancestor-or-self:: following:: preceding:: following-sibling:: preceding-sibling::";

#[derive(Clone, Debug, Default)]
pub(super) struct State {
    scan: Scan,
    stack: Vec<Frame>,
}

#[derive(Clone, Copy, Debug, Default)]
enum Scan {
    #[default]
    Base,
    Attribute,
    String {
        quote: char,
        attribute: bool,
    },
    Comment,
    XmlComment,
    Cdata,
    Processing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Frame {
    // These frames all restore the base scanner when exposed by a pop.
    Scope,
    Tag,
    Constructor,
    Attribute,
    String { quote: char, attribute: bool },
}

impl State {
    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        if stream.eat_space() {
            return None;
        }
        match self.scan {
            Scan::Base => self.base(stream),
            Scan::Attribute => self.attribute(stream),
            Scan::String { quote, attribute } => Some(self.string(stream, quote, attribute)),
            Scan::Comment => Some(self.comment(stream)),
            Scan::XmlComment => self.until(stream, "-->", "comment"),
            Scan::Cdata => self.until(stream, "]]", "comment"),
            Scan::Processing => self.until(stream, "?>", "processingInstruction"),
        }
    }

    fn base(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        let ch = stream.next()?;
        let eqname = eqname_ahead(ch, &stream.line()[stream.pos()..]);
        match ch {
            '<' => self.tag(stream),
            '{' => {
                self.stack.push(Frame::Scope);
                None
            }
            '}' => {
                self.pop();
                None
            }
            _ if self.inside(Frame::Tag) => Some(match ch {
                '>' => "tag",
                '/' if stream.eat('>').is_some() => {
                    self.pop();
                    "tag"
                }
                _ => "variable",
            }),
            ch if ch.is_ascii_digit() => {
                number_tail(stream);
                Some("atom")
            }
            '(' if stream.eat(':').is_some() => {
                self.stack.push(Frame::Scope);
                self.scan = Scan::Comment;
                Some(self.comment(stream))
            }
            '"' | '\'' if !eqname => Some(self.start_string(stream, ch, false)),
            '$' => Some(variable(stream)),
            ':' if stream.eat('=').is_some() => Some("keyword"),
            '(' | '[' => {
                self.stack.push(Frame::Scope);
                None
            }
            ')' | ']' => {
                self.pop();
                None
            }
            _ => self.word(stream, ch, eqname),
        }
    }

    fn word(
        &mut self,
        stream: &mut StringStream<'_>,
        ch: char,
        eqname: bool,
    ) -> Option<&'static str> {
        if eqname {
            take_quote(stream, ch);
        }
        if !",=><.|?*/+-".contains(ch) {
            stream.eat_while(name_char);
        }
        let colon = stream.eat(':').is_some();
        if stream.eat(':').is_none() && colon {
            stream.eat_while(name_char);
        }
        let text = stream.current();
        if self.inside(Frame::Constructor) {
            self.pop();
            return Some("variable");
        }
        if contains(AXES, text) {
            self.stack.push(Frame::Constructor);
            return Some("qualifier");
        }
        if matches!(text, "element" | "attribute") {
            self.stack.push(Frame::Constructor);
        }
        if text == "," {
            None
        } else if contains(TYPES, text) {
            Some("atom")
        } else if contains(KEYWORDS, text) || contains(OPERATORS, text) {
            Some("keyword")
        } else if stream.line()[stream.pos()..]
            .trim_start_matches([' ', '\t'])
            .starts_with('(')
        {
            Some("def")
        } else {
            Some("variable")
        }
    }

    fn tag(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        if stream.match_str("!--", true, false) {
            self.scan = Scan::XmlComment;
            return self.until(stream, "-->", "comment");
        }
        if stream.match_str("![CDATA", false, false) {
            self.scan = Scan::Cdata;
            return Some("tag");
        }
        if stream.peek() == Some('?') {
            self.scan = Scan::Processing;
            return self.until(stream, "?>", "processingInstruction");
        }
        let closing = stream.eat('/').is_some();
        stream.eat_space();
        stream.eat_while(|ch| !js_space(ch) && !"=<>\"'/?".contains(ch));
        stream.eat_space();
        if closing && stream.eat('>').is_some() {
            self.pop();
            self.scan = Scan::Base;
        } else {
            if stream.eat('/').is_none() {
                self.stack.push(Frame::Tag);
            }
            self.scan = if stream.eat('>').is_some() {
                Scan::Base
            } else {
                Scan::Attribute
            };
        }
        Some("tag")
    }

    fn attribute(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        match stream.next()? {
            '/' if stream.eat('>').is_some() => {
                if self.inside(Frame::Attribute) {
                    self.pop();
                }
                if self.inside(Frame::Tag) {
                    self.pop();
                }
                Some("tag")
            }
            '>' => {
                if self.inside(Frame::Attribute) {
                    self.pop();
                }
                Some("tag")
            }
            '=' => None,
            ch @ ('"' | '\'') => Some(self.start_string(stream, ch, true)),
            _ => {
                if !self.inside(Frame::Attribute) {
                    self.stack.push(Frame::Attribute);
                }
                stream.eat_if(|ch| ch.is_ascii_alphabetic() || matches!(ch, '_' | ':'));
                stream.eat_while(|ch| word(ch) || matches!(ch, '-' | ':' | '.'));
                stream.eat_space();
                if matches!(stream.peek(), Some('>' | '/')) {
                    self.pop();
                    self.scan = Scan::Base;
                }
                Some("attribute")
            }
        }
    }

    fn start_string(
        &mut self,
        stream: &mut StringStream<'_>,
        quote: char,
        attribute: bool,
    ) -> &'static str {
        self.stack.push(Frame::String { quote, attribute });
        self.scan = Scan::String { quote, attribute };
        self.string(stream, quote, attribute)
    }

    fn string(
        &mut self,
        stream: &mut StringStream<'_>,
        quote: char,
        attribute: bool,
    ) -> &'static str {
        if take_quote(stream, quote) {
            self.pop();
            if attribute {
                self.scan = Scan::Attribute;
            }
        }
        "string"
    }

    fn comment(&mut self, stream: &mut StringStream<'_>) -> &'static str {
        // The upstream scanner counts nested comments only within this call.
        let mut nesting = 0usize;
        let mut last = None;
        for ch in stream {
            match (last, ch) {
                (Some(':'), ')') if nesting == 0 => {
                    self.pop();
                    break;
                }
                (Some(':'), ')') => nesting -= 1,
                (Some('('), ':') => nesting += 1,
                _ => {}
            }
            last = Some(ch);
        }
        "comment"
    }

    fn until(
        &mut self,
        stream: &mut StringStream<'_>,
        end: &str,
        style: &'static str,
    ) -> Option<&'static str> {
        if stream.skip_to(end) {
            stream.match_str(end, true, false);
            self.scan = Scan::Base;
            Some(style)
        } else {
            stream.skip_to_end();
            None
        }
    }

    fn inside(&self, frame: Frame) -> bool {
        self.stack.last() == Some(&frame)
    }

    fn pop(&mut self) {
        self.stack.pop();
        self.scan = match self.stack.last() {
            Some(Frame::Attribute) => Scan::Attribute,
            Some(&Frame::String { quote, attribute }) => Scan::String { quote, attribute },
            _ => Scan::Base,
        };
    }
}

fn eqname_ahead(ch: char, rest: &str) -> bool {
    if ch == '"' {
        rest.find('"')
            .is_some_and(|end| end > 0 && rest[end + 1..].starts_with(':'))
    } else if ch == '\'' {
        rest.split('"')
            .next()
            .and_then(|text| text.match_indices("':").find(|(pos, _)| *pos > 0))
            .is_some()
    } else {
        false
    }
}

fn variable(stream: &mut StringStream<'_>) -> &'static str {
    if stream.eat('"').is_some() {
        // Upstream assumes a closing quote. Stop at EOL on malformed input.
        take_quote(stream, '"');
        stream.eat(':');
    } else {
        stream.eat_while(name_char);
        if !stream.match_str(":=", false, false) {
            stream.eat(':');
        }
    }
    stream.eat_while(name_char);
    "variable"
}

fn number_tail(stream: &mut StringStream<'_>) {
    stream.eat_while(|ch| ch.is_ascii_digit());
    if stream.eat('.').is_some() {
        stream.eat_while(|ch| ch.is_ascii_digit());
    }
    let start = stream.pos();
    if stream.eat('E').is_some() {
        stream.eat_if(|ch| matches!(ch, '+' | '-'));
        if !stream.eat_while(|ch| ch.is_ascii_digit()) {
            stream
                .set_pos(start)
                .expect("exponent starts at a scalar boundary");
        }
    }
}

fn take_quote(stream: &mut StringStream<'_>, quote: char) -> bool {
    stream.any(|ch| ch == quote)
}

fn name_char(ch: char) -> bool {
    word(ch) || matches!(ch, '$' | '-')
}

fn contains(words: &str, word: &str) -> bool {
    words.split_ascii_whitespace().any(|entry| entry == word)
}
