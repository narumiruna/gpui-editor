use super::prefix;
use crate::stream_parser::StringStream;

// In the upstream UTF-16 expression, both halves of a surrogate pair match the range.
const LETTERS: &str = concat!(r"[$\w\xAA-\uFFDC", "\u{10000}-\u{10ffff}", "]");
const KEYWORD_END: &str = r"(?![$\w]|-[A-Za-z]|\s*:(?![:=]))";
const KEYWORDS: &str = concat!(
    r"(?:t(?:h(?:is|row|en)|ry|ypeof!?)|c(?:on(?:tinue|st)|a(?:se|tch)|lass)|",
    r"i(?:n(?:stanceof)?|mp(?:ort(?:\s+all)?|lements)|[fs])|d(?:e(?:fault|lete|bugger)|o)|",
    r"f(?:or(?:\s+own)?|inally|unction)|s(?:uper|witch)|e(?:lse|x(?:tends|port)|val)|",
    r"a(?:nd|rguments)|n(?:ew|ot)|un(?:less|til)|w(?:hile|ith)|o[fr]|return|break|let|var|loop)",
);

fn identifier() -> String {
    format!(r"(?![\d\s]){LETTERS}(?:(?!\s){LETTERS}|-[A-Za-z])*")
}

macro_rules! rule {
    ($self:ident, $stream:ident, $pattern:expr, $style:literal $(, $next:ident)?) => {
        if prefix($stream, re!($pattern)) {
            $($self.scan = Scan::$next;)?
            return Some($style);
        }
    };
}

#[derive(Clone, Copy, Debug, Default)]
enum Scan {
    #[default]
    Start,
    Heregex,
    Key,
    Comment,
    SingleDoc,
    DoubleDoc,
    SingleString,
    DoubleString,
    Script,
    Words,
}

#[derive(Clone, Debug, Default)]
pub(super) struct State {
    scan: Scan,
    indentation: usize,
}

impl State {
    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> &'static str {
        let style = self.scan(stream).unwrap_or_else(|| {
            stream.next();
            "error"
        });
        self.indentation = stream.indentation();
        let indenter = re!(&format!(
            r"(?:[({{[=:]|[-~]>|\b(?:e(?:lse|xport)|d(?:o|efault)|t(?:ry|hen)|finally|import(?:\s*all)?|const|var|let|new|catch(?:\s*{})?))\s*$",
            identifier(),
        ));
        if indenter.find(stream.current()).is_some() {
            self.indentation = self.indentation.saturating_add(2);
        }
        style
    }

    pub(super) fn indent(&self) -> usize {
        self.indentation
    }

    fn scan(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        match self.scan {
            Scan::Start => return self.start(stream),
            Scan::Key => {
                rule!(self, stream, r"^[.?@!]+", "operatorKeyword");
                rule!(
                    self,
                    stream,
                    &format!("^{}", identifier()),
                    "variableName",
                    Start
                );
                // The last key rule consumes nothing and returns to the start rules.
                self.scan = Scan::Start;
                return self.start(stream);
            }
            Scan::Heregex => {
                rule!(self, stream, r"^.*?//[gimy$?]{0,4}", "regexp", Start);
                rule!(self, stream, r"^\s*#{", "regexp");
                rule!(self, stream, r"^\s+(?:#.*)?", "comment");
                rule!(self, stream, r"^\S+", "regexp");
                return None;
            }
            Scan::Comment => {
                rule!(self, stream, r"^.*?\*/", "docComment", Start);
                rule!(self, stream, r"^.+", "docComment");
                return None;
            }
            Scan::SingleDoc => {
                rule!(self, stream, r"^.*?'''", "string", Key);
            }
            Scan::DoubleDoc => {
                rule!(self, stream, r#"^.*?""""#, "string", Key);
            }
            Scan::SingleString => {
                rule!(self, stream, r"^[^\\']*(?:\\.[^\\']*)*'", "string", Key);
            }
            Scan::DoubleString => {
                rule!(self, stream, r#"^[^\\"]*(?:\\.[^\\"]*)*""#, "string", Key);
            }
            Scan::Script => {
                rule!(self, stream, r"^[^\\`]*(?:\\.[^\\`]*)*`", "string", Key);
            }
            Scan::Words => {
                rule!(self, stream, r"^.*?\]>", "string", Key);
            }
        }
        rule!(self, stream, r"^.+", "string");
        None
    }

    fn start(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        rule!(self, stream, r"^/\*", "docComment", Comment);
        rule!(self, stream, r"^#.*", "comment");
        rule!(
            self,
            stream,
            &format!("^{KEYWORDS}{KEYWORD_END}"),
            "keyword"
        );
        rule!(
            self,
            stream,
            &format!(r"^(?:true|false|yes|no|on|off|null|void|undefined){KEYWORD_END}"),
            "atom"
        );
        rule!(
            self,
            stream,
            &format!(
                r"^(?:p(?:ackage|r(?:ivate|otected)|ublic)|i(?:mplements|nterface)|enum|static|yield){KEYWORD_END}"
            ),
            "invalid"
        );
        rule!(
            self,
            stream,
            &format!(
                r"^(?:R(?:e(?:gExp|ferenceError)|angeError)|S(?:tring|yntaxError)|E(?:rror|valError)|Array|Boolean|Date|Function|Number|Object|TypeError|URIError){KEYWORD_END}"
            ),
            "className standard"
        );
        rule!(
            self,
            stream,
            &format!(
                r"^(?:is(?:NaN|Finite)|parse(?:Int|Float)|Math|JSON|(?:en|de)codeURI(?:Component)?){KEYWORD_END}"
            ),
            "variableName function standard"
        );
        rule!(
            self,
            stream,
            &format!(r"^(?:t(?:hat|il|o)|f(?:rom|allthrough)|it|by|e){KEYWORD_END}"),
            "variableName standard"
        );
        rule!(
            self,
            stream,
            &format!(r"^{}\s*:(?![:=])", identifier()),
            "variableName"
        );
        rule!(self, stream, &format!("^{}", identifier()), "variableName");
        rule!(self, stream, r"^(?:\.{3}|\s+\?)", "operatorKeyword");
        rule!(self, stream, r"^(?:@+|::|\.\.)", "keyword", Key);
        rule!(self, stream, r"^\.\s*", "operatorKeyword", Key);
        rule!(self, stream, r"^\\\S[^\s,;)}\]]*", "string");
        rule!(self, stream, r"^'''", "docString", SingleDoc);
        rule!(self, stream, r#"^""""#, "docString", DoubleDoc);
        rule!(self, stream, r"^'", "string", SingleString);
        rule!(self, stream, r#"^""#, "string", DoubleString);
        rule!(self, stream, r"^`", "string", Script);
        rule!(self, stream, r"^<\[", "string", Words);
        rule!(self, stream, r"^//", "regexp", Heregex);
        rule!(
            self,
            stream,
            r"^\/(?:[^[\/\n\\]*(?:(?:\\.|\[[^\]\n\\]*(?:\\.[^\]\n\\]*)*\])[^[\/\n\\]*)*)\/[gimy$]{0,4}",
            "regexp",
            Key
        );
        rule!(
            self,
            stream,
            r"^(?:0x[\da-fA-F][\da-fA-F_]*|(?:[2-9]|[12]\d|3[0-6])r[\da-zA-Z][\da-zA-Z_]*|(?:\d[\d_]*(?:\.\d[\d_]*)?|\.\d[\d_]*)(?:e[+-]?\d[\d_]*)?[\w$]*)",
            "number"
        );
        rule!(self, stream, r"^[({[]", "paren");
        rule!(self, stream, r"^[)}\]]", "paren", Key);
        rule!(self, stream, r"^\S+", "operatorKeyword");
        rule!(self, stream, r"^\s+", "content");
        None
    }
}
