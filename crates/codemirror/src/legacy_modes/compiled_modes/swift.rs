use std::sync::LazyLock;

use regex::Regex;

use super::{in_words, js_space, remaining, take};
use crate::stream_parser::StringStream;

const KEYWORDS: &str = "_ var let actor class enum extension import protocol struct func typealias associatedtype \
    open public internal fileprivate private deinit init new override self subscript super convenience dynamic \
    final indirect lazy required static unowned unowned(safe) unowned(unsafe) weak as is break case continue \
    default else fallthrough for guard if in repeat switch where while defer return inout mutating nonmutating \
    isolated nonisolated catch do rethrows throw throws async await try didSet get set willSet assignment \
    associativity infix left none operator postfix precedence precedencegroup prefix right Any AnyObject Type \
    dynamicType Self Protocol __COLUMN__ __FILE__ __FUNCTION__ __LINE__";
const DEFINING: &str =
    "var let actor class enum extension import protocol struct func typealias associatedtype for";
const ATOMS: &str = "true false nil self super _";
const TYPES: &str = "Array Bool Character Dictionary Double Float Int Int8 Int16 Int32 Int64 Never Optional \
    Set String UInt8 UInt16 UInt32 UInt64 Void";
const OPERATORS: &str = "+-/*%=|&<>~^?!";
const PUNCTUATION: &str = ":;,.(){}[]";

static NUMBERS: LazyLock<[Regex; 4]> = LazyLock::new(|| {
    [
        r"^-?0b[01][01_]*",
        r"^-?0o[0-7][0-7_]*",
        r"^-?0x[0-9A-Fa-f][0-9A-Fa-f_]*(?:(?:\.[0-9A-Fa-f][0-9A-Fa-f_]*)?[Pp]-?[0-9][0-9_]*)?",
        r"^-?[0-9][0-9_]*(?:\.[0-9][0-9_]*)?(?:[Ee]-?[0-9][0-9_]*)?",
    ]
    .map(|pattern| Regex::new(pattern).unwrap())
});
static IDENTIFIER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:\$[0-9]+|`[_A-Za-z][_A-Za-z$0-9]*`|[_A-Za-z][_A-Za-z$0-9]*)").unwrap()
});

#[derive(Clone, Copy, Debug)]
enum Scan {
    String(&'static str),
    Interpolation(usize),
    Comment,
}

#[derive(Clone, Copy, Debug)]
struct Context {
    align: Option<usize>,
    indented: usize,
}

#[derive(Clone, Debug, Default)]
pub(super) struct State {
    previous: Option<&'static str>,
    contexts: Vec<Context>,
    indented: usize,
    scans: Vec<Scan>,
}

impl State {
    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        let previous = self.previous.take();
        let style = match self.scans.last().copied() {
            None => self.base(stream, previous),
            Some(Scan::String(quote)) => Some(self.string(stream, quote)),
            Some(Scan::Comment) => Some(self.comment(stream)),
            Some(Scan::Interpolation(depth)) => self.interpolation(stream, previous, depth),
        };
        if matches!(style, Some("comment") | None) {
            self.previous = previous;
        } else if self.previous.is_none() {
            self.previous = style;
        }
        if style == Some("punctuation") {
            match stream.current().as_bytes().first() {
                Some(b'(' | b'[' | b'{') => {
                    let rest = remaining(stream).trim_start_matches(js_space);
                    let align = (!rest.is_empty()
                        && !rest.starts_with([')', '}', ']'])
                        && !rest.starts_with("//")
                        && !rest.starts_with("/*"))
                    .then(|| stream.column().saturating_add(1));
                    self.contexts.push(Context {
                        align,
                        indented: self.indented,
                    });
                }
                Some(b']' | b')' | b'}') => {
                    if let Some(context) = self.contexts.pop() {
                        self.indented = context.indented;
                    }
                }
                _ => {}
            }
        }
        style
    }

    pub(super) fn indent(&self, text_after: &str, unit: usize) -> usize {
        let Some(context) = self.contexts.last() else {
            return 0;
        };
        let closing = text_after.starts_with([']', '}', ')']);
        match context.align {
            Some(align) => align - usize::from(closing),
            None => context
                .indented
                .saturating_add(if closing { 0 } else { unit }),
        }
    }

    fn base(
        &mut self,
        stream: &mut StringStream<'_>,
        previous: Option<&str>,
    ) -> Option<&'static str> {
        if stream.sol() {
            self.indented = stream.indentation();
        }
        if stream.eat_space() {
            return None;
        }
        let ch = stream.peek()?;
        match ch {
            '/' if take(stream, "//") => {
                stream.skip_to_end();
                return Some("comment");
            }
            '/' if take(stream, "/*") => {
                self.scans.push(Scan::Comment);
                return Some(self.comment(stream));
            }
            '#' if remaining(stream)
                .as_bytes()
                .get(1)
                .is_some_and(u8::is_ascii_alphabetic) =>
            {
                stream.next();
                stream.eat_while(|ch| ch.is_ascii_alphabetic());
                return Some("builtin");
            }
            '@' if prefixed_identifier(stream) => return Some("attribute"),
            _ => {}
        }
        if (ch.is_ascii_digit() || ch == '-')
            && NUMBERS
                .iter()
                .any(|pattern| stream.match_regex(pattern, true).is_some())
        {
            return Some("number");
        }
        if ch == '.' && prefixed_identifier(stream) {
            return Some("property");
        }
        if OPERATORS.contains(ch) {
            stream.next();
            return Some("operator");
        }
        if PUNCTUATION.contains(ch) {
            stream.next();
            take(stream, "..");
            return Some("punctuation");
        }
        if let Some(quote) = ["\"\"\"", "\"", "'"]
            .into_iter()
            .find(|quote| take(stream, quote))
        {
            self.scans.push(Scan::String(quote));
            return Some(self.string(stream, quote));
        }
        if stream.match_regex(&IDENTIFIER, true).is_some() {
            let id = stream.current();
            return Some(if in_words(TYPES, id) {
                "type"
            } else if in_words(ATOMS, id) {
                "atom"
            } else if in_words(KEYWORDS, id) {
                if in_words(DEFINING, id) {
                    self.previous = Some("define");
                }
                "keyword"
            } else if previous == Some("define") {
                "def"
            } else {
                "variable"
            });
        }
        stream.next();
        None
    }

    fn interpolation(
        &mut self,
        stream: &mut StringStream<'_>,
        previous: Option<&str>,
        depth: usize,
    ) -> Option<&'static str> {
        let index = self.scans.len() - 1;
        let style = self.base(stream, previous);
        if style == Some("punctuation") {
            match stream.current() {
                "(" => self.scans[index] = Scan::Interpolation(depth + 1),
                ")" if depth > 0 => self.scans[index] = Scan::Interpolation(depth - 1),
                ")" => {
                    stream.back_up(1).unwrap();
                    self.scans.pop();
                    if let Some(Scan::String(quote)) = self.scans.last().copied() {
                        return Some(self.string(stream, quote));
                    }
                }
                _ => {}
            }
        }
        style
    }

    fn string(&mut self, stream: &mut StringStream<'_>, quote: &'static str) -> &'static str {
        while let Some(ch) = stream.peek() {
            if take(stream, quote) {
                self.scans.pop();
                return "string";
            }
            stream.next();
            if ch == '\\' && stream.next() == Some('(') {
                self.scans.push(Scan::Interpolation(0));
                return "string";
            }
        }
        if quote.len() == 1 {
            self.scans.pop();
        }
        "string"
    }

    fn comment(&mut self, stream: &mut StringStream<'_>) -> &'static str {
        while let Some(ch) = stream.next() {
            if ch == '/' && stream.eat('*').is_some() {
                self.scans.push(Scan::Comment);
            } else if ch == '*' && stream.eat('/').is_some() {
                self.scans.pop();
                break;
            }
        }
        "comment"
    }
}

fn prefixed_identifier(stream: &mut StringStream<'_>) -> bool {
    stream.next();
    if stream.match_regex(&IDENTIFIER, true).is_some() {
        true
    } else {
        stream.back_up(1).unwrap();
        false
    }
}

pub(super) fn indent_on_input(text: &str) -> bool {
    matches!(text.trim_start_matches(js_space), ")" | "}" | "]")
}
