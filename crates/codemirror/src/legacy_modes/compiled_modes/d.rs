use super::{in_words, js_space, word};
use crate::stream_parser::StringStream;

const KEYWORDS: &str = "abstract alias align asm assert auto break case cast cdouble cent cfloat const continue \
    debug default delegate delete deprecated export extern final finally function goto immutable import inout \
    invariant is lazy macro module new nothrow override package pragma private protected public pure ref return \
    shared short static super synchronized template this throw typedef typeid typeof volatile __FILE__ __LINE__ \
    __gshared __traits __vector __parameters";
const BLOCK_KEYWORDS: &str = "body catch class do else enum for foreach foreach_reverse if in interface mixin \
    out scope struct switch try union unittest version while with";
const BUILTINS: &str = "bool byte char creal dchar double float idouble ifloat int ireal long real short ubyte \
    ucent uint ulong ushort wchar wstring void size_t sizediff_t";
const ATOMS: &str = "exit failure success true false null";
const OPERATORS: &str = "+-*&%=<>!?|/";

#[derive(Clone, Copy, Debug)]
enum Scan {
    String(char),
    Comment(char),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Top,
    Statement,
    Delimiter(char),
}

#[derive(Clone, Copy, Debug)]
struct Context {
    kind: Kind,
    align: Option<bool>,
    column: usize,
    indented: usize,
}

#[derive(Clone, Debug)]
pub(super) struct State {
    scan: Option<Scan>,
    contexts: Vec<Context>,
    indented: usize,
    top_indent_unit: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mark {
    Punctuation(char),
    NewStatement,
}

impl State {
    pub(super) fn new(indent_unit: usize) -> Self {
        Self {
            scan: None,
            contexts: vec![Context {
                kind: Kind::Top,
                align: Some(false),
                column: 0,
                indented: 0,
            }],
            indented: 0,
            top_indent_unit: indent_unit,
        }
    }

    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        let index = self.contexts.len() - 1;
        if stream.sol() {
            self.contexts[index].align.get_or_insert(false);
            self.indented = stream.indentation();
        }
        if stream.eat_space() {
            return None;
        }
        let mut mark = None;
        let style = if let Some(scan) = self.scan {
            Some(self.continued(stream, scan))
        } else {
            self.base(stream, &mut mark)
        };
        if matches!(style, Some("comment" | "meta")) {
            return style;
        }
        self.contexts[index].align.get_or_insert(true);
        let kind = self.contexts[index].kind;
        match mark {
            Some(Mark::Punctuation(';' | ':' | ',')) if kind == Kind::Statement => self.pop(),
            Some(Mark::Punctuation('{')) => self.push(stream, Kind::Delimiter('}')),
            Some(Mark::Punctuation('[')) => self.push(stream, Kind::Delimiter(']')),
            Some(Mark::Punctuation('(')) => self.push(stream, Kind::Delimiter(')')),
            Some(Mark::Punctuation('}')) => {
                self.pop_statements();
                if self.context().kind == Kind::Delimiter('}') {
                    self.pop();
                }
                self.pop_statements();
            }
            Some(Mark::Punctuation(ch)) if kind == Kind::Delimiter(ch) => self.pop(),
            _ if (matches!(kind, Kind::Top | Kind::Delimiter('}'))
                && mark != Some(Mark::Punctuation(';')))
                || (kind == Kind::Statement && mark == Some(Mark::NewStatement)) =>
            {
                self.push(stream, Kind::Statement)
            }
            _ => {}
        }
        style
    }

    pub(super) fn indent(&self, text_after: &str, unit: usize) -> Option<usize> {
        if self.scan.is_some() {
            return None;
        }
        let mut index = self.contexts.len() - 1;
        let first = text_after.chars().next();
        if self.contexts[index].kind == Kind::Statement && first == Some('}') {
            index -= 1;
        }
        let context = self.contexts[index];
        let closing = first.is_some_and(|ch| context.kind == Kind::Delimiter(ch));
        Some(match context.kind {
            Kind::Top => unit.saturating_sub(self.top_indent_unit),
            Kind::Statement => {
                context
                    .indented
                    .saturating_add(if first == Some('{') { 0 } else { unit })
            }
            Kind::Delimiter(_) if context.align == Some(true) => {
                context.column.saturating_add(usize::from(!closing))
            }
            Kind::Delimiter(_) => context
                .indented
                .saturating_add(if closing { 0 } else { unit }),
        })
    }

    fn base(
        &mut self,
        stream: &mut StringStream<'_>,
        mark: &mut Option<Mark>,
    ) -> Option<&'static str> {
        let ch = stream.next()?;
        Some(match ch {
            '@' => {
                stream.eat_while(|ch| word(ch) || ch == '$');
                "meta"
            }
            '"' | '\'' | '`' => self.continued(stream, Scan::String(ch)),
            '[' | ']' | '{' | '}' | '(' | ')' | ',' | ';' | ':' | '.' => {
                *mark = Some(Mark::Punctuation(ch));
                return None;
            }
            '0'..='9' => {
                stream.eat_while(|ch| word(ch) || ch == '.');
                "number"
            }
            '/' if stream.eat('+').is_some() => self.continued(stream, Scan::Comment('+')),
            '/' if stream.eat('*').is_some() => self.continued(stream, Scan::Comment('*')),
            '/' if stream.eat('/').is_some() => {
                stream.skip_to_end();
                "comment"
            }
            ch if OPERATORS.contains(ch) => {
                stream.eat_while(|ch| OPERATORS.contains(ch));
                "operator"
            }
            _ => {
                stream.eat_while(|ch| word(ch) || ch == '$' || ch >= '\u{a1}');
                let id = stream.current();
                if in_words(BLOCK_KEYWORDS, id) {
                    *mark = Some(Mark::NewStatement);
                    "keyword"
                } else if in_words(KEYWORDS, id) {
                    "keyword"
                } else if in_words(BUILTINS, id) {
                    "builtin"
                } else if in_words(ATOMS, id) {
                    "atom"
                } else {
                    "variable"
                }
            }
        })
    }

    fn continued(&mut self, stream: &mut StringStream<'_>, scan: Scan) -> &'static str {
        match scan {
            Scan::String(quote) => {
                let mut escaped = false;
                let mut ended = false;
                for ch in stream {
                    if ch == quote && !escaped {
                        ended = true;
                        break;
                    }
                    escaped = !escaped && ch == '\\';
                }
                self.scan = (escaped && !ended).then_some(scan);
                "string"
            }
            Scan::Comment(marker) => {
                // The pinned D scanner ends /+ comments at the first +/.
                let mut maybe_end = false;
                self.scan = Some(scan);
                for ch in stream {
                    if ch == '/' && maybe_end {
                        self.scan = None;
                        break;
                    }
                    maybe_end = ch == marker;
                }
                "comment"
            }
        }
    }

    fn context(&self) -> Context {
        *self.contexts.last().unwrap()
    }

    fn push(&mut self, stream: &StringStream<'_>, kind: Kind) {
        let parent = self.context();
        self.contexts.push(Context {
            kind,
            align: None,
            column: stream.column(),
            indented: if parent.kind == Kind::Statement {
                parent.indented
            } else {
                self.indented
            },
        });
    }

    fn pop(&mut self) {
        if let Some(context) = self.contexts.pop()
            && matches!(context.kind, Kind::Delimiter(_))
        {
            self.indented = context.indented;
        }
    }

    fn pop_statements(&mut self) {
        while self.context().kind == Kind::Statement {
            self.pop();
        }
    }
}

pub(super) fn indent_on_input(text: &str) -> bool {
    matches!(text.trim_start_matches(js_space), "{" | "}")
}
