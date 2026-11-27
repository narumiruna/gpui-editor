use super::{StringStream, ascii_word};

const KEYWORDS: &[&str] = &[
    "abstract",
    "as",
    "assert",
    "boolean",
    "break",
    "byte",
    "case",
    "catch",
    "char",
    "class",
    "const",
    "continue",
    "def",
    "default",
    "do",
    "double",
    "else",
    "enum",
    "extends",
    "final",
    "finally",
    "float",
    "for",
    "goto",
    "if",
    "implements",
    "import",
    "in",
    "instanceof",
    "int",
    "interface",
    "long",
    "native",
    "new",
    "package",
    "private",
    "protected",
    "public",
    "return",
    "short",
    "static",
    "strictfp",
    "super",
    "switch",
    "synchronized",
    "threadsafe",
    "throw",
    "throws",
    "trait",
    "transient",
    "try",
    "void",
    "volatile",
    "while",
];
const BLOCK: &[&str] = &[
    "catch",
    "class",
    "def",
    "do",
    "else",
    "enum",
    "finally",
    "for",
    "if",
    "interface",
    "switch",
    "trait",
    "try",
    "while",
];

type Lex = (Option<&'static str>, Option<&'static str>);

fn styled(style: &'static str) -> Lex {
    (Some(style), None)
}

#[derive(Clone, Debug)]
pub(super) struct State {
    frames: Vec<Frame>,
    contexts: Vec<Context>,
    root_unit: usize,
    indented: usize,
    last: Option<&'static str>,
}

#[derive(Clone, Copy, Debug)]
enum Frame {
    String { quote: char, triple: bool },
    Brace(usize),
    Deref,
    Comment,
}

#[derive(Clone, Copy, Debug)]
struct Context {
    kind: &'static str,
    indented: usize,
    column: usize,
    align: Option<bool>,
}

impl State {
    pub(super) fn new(unit: usize) -> Self {
        Self {
            frames: Vec::new(),
            contexts: vec![Context {
                kind: "top",
                indented: 0,
                column: 0,
                align: Some(false),
            }],
            root_unit: unit,
            indented: 0,
            last: None,
        }
    }

    fn top(&self) -> Context {
        *self.contexts.last().unwrap()
    }

    fn push(&mut self, kind: &'static str, column: usize) {
        self.contexts.push(Context {
            kind,
            indented: self.indented,
            column,
            align: None,
        });
    }

    fn pop(&mut self) {
        let context = self.contexts.pop().unwrap();
        if matches!(context.kind, ")" | "]" | "}") {
            self.indented = context.indented;
        }
    }

    fn base(&mut self, stream: &mut StringStream<'_>) -> Lex {
        let Some(ch) = stream.next() else {
            return (None, None);
        };
        let punc = match ch {
            '[' => Some("["),
            ']' => Some("]"),
            '{' => Some("{"),
            '}' => Some("}"),
            '(' => Some("("),
            ')' => Some(")"),
            ',' => Some(","),
            ';' => Some(";"),
            ':' => Some(":"),
            '.' => Some("."),
            _ => None,
        };
        if punc.is_some() {
            return (None, punc);
        }
        if matches!(ch, '\'' | '"') {
            return self.start_string(stream, ch);
        }
        if ch.is_ascii_digit() {
            stream.eat_while(|ch| ascii_word(ch) || ch == '.');
            return styled("number");
        }
        if ch == '/' {
            if stream.eat('*').is_some() {
                self.frames.push(Frame::Comment);
                return self.comment(stream);
            }
            if stream.eat('/').is_some() {
                stream.skip_to_end();
                return styled("comment");
            }
            if expect_expression(self.last, false) {
                return self.start_string(stream, ch);
            }
        }
        if ch == '-' && stream.eat('>').is_some() {
            return (None, Some("->"));
        }
        if "+-*&%=<>!?|/~".contains(ch) {
            stream.eat_while(|ch| "+-*&%=<>|~".contains(ch));
            return styled("operator");
        }
        stream.eat_while(word);
        if ch == '@' {
            stream.eat_while(|ch| word(ch) || ch == '.');
            return styled("meta");
        }
        if self.last == Some(".") {
            return styled("property");
        }
        if stream.eat(':').is_some() {
            return (Some("property"), Some("proplabel"));
        }
        let current = stream.current();
        if matches!(current, "null" | "true" | "false" | "this") {
            return styled("atom");
        }
        if KEYWORDS.contains(&current) {
            return (
                Some("keyword"),
                if BLOCK.contains(&current) {
                    Some("newstatement")
                } else if matches!(current, "return" | "break" | "continue") {
                    Some("standalone")
                } else {
                    None
                },
            );
        }
        styled("variable")
    }

    fn start_string(&mut self, stream: &mut StringStream<'_>, quote: char) -> Lex {
        let mut triple = false;
        if quote != '/' && stream.eat(quote).is_some() {
            if stream.eat(quote).is_none() {
                return styled("string");
            }
            triple = true;
        }
        self.frames.push(Frame::String { quote, triple });
        self.string(stream, quote, triple)
    }

    fn string(&mut self, stream: &mut StringStream<'_>, quote: char, triple: bool) -> Lex {
        let triple_end = if quote == '"' { "\"\"" } else { "''" };
        let mut escaped = false;
        while let Some(ch) = stream.next() {
            if ch == quote && !escaped && (!triple || super::eat_str(stream, triple_end)) {
                self.frames.pop();
                return styled("string");
            }
            if quote == '"' && ch == '$' && !escaped {
                if stream.eat('{').is_some() {
                    self.frames.push(Frame::Brace(1));
                    return styled("string");
                }
                if stream.peek().is_some_and(ascii_word) {
                    self.frames.push(Frame::Deref);
                    return styled("string");
                }
            }
            escaped = !escaped && ch == '\\';
        }
        if !triple {
            self.frames.pop();
        }
        styled("string")
    }

    fn comment(&mut self, stream: &mut StringStream<'_>) -> Lex {
        if stream.skip_to("*/") {
            super::eat_str(stream, "*/");
            self.frames.pop();
        } else {
            stream.skip_to_end();
        }
        styled("comment")
    }

    fn lex(&mut self, stream: &mut StringStream<'_>) -> Lex {
        loop {
            match self.frames.last_mut() {
                None => return self.base(stream),
                Some(Frame::String { quote, triple }) => {
                    let (quote, triple) = (*quote, *triple);
                    return self.string(stream, quote, triple);
                }
                Some(Frame::Brace(depth)) => match stream.peek() {
                    Some('}') if *depth == 1 => {
                        self.frames.pop();
                    }
                    next => {
                        match next {
                            Some('{') => *depth += 1,
                            Some('}') => *depth -= 1,
                            _ => (),
                        }
                        return self.base(stream);
                    }
                },
                Some(Frame::Deref) => {
                    if stream.eat('.').is_some() {
                        if stream.eat_if(word).is_none() {
                            self.frames.pop();
                        }
                        return (None, None);
                    }
                    if stream.eat_while(word) {
                        if stream.eat('.').is_none() {
                            self.frames.pop();
                        }
                        return styled("variable");
                    }
                    self.frames.pop();
                }
                Some(Frame::Comment) => return self.comment(stream),
            }
        }
    }

    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        if stream.sol() {
            self.contexts.last_mut().unwrap().align.get_or_insert(false);
            self.indented = stream.indentation();
            if self.top().kind == "statement" && !expect_expression(self.last, true) {
                self.pop();
            }
        }
        if stream.eat_space() {
            return None;
        }
        let (style, punc) = self.lex(stream);
        if style == Some("comment") {
            return style;
        }
        self.contexts.last_mut().unwrap().align.get_or_insert(true);
        let ctx = self.top();
        let column = stream.column();
        match punc {
            Some(";" | ":") if ctx.kind == "statement" => self.pop(),
            Some("->")
                if ctx.kind == "statement"
                    && self.contexts[self.contexts.len() - 2].kind == "}" =>
            {
                self.pop();
                self.contexts.last_mut().unwrap().align = Some(false);
            }
            Some("{") => self.push("}", column),
            Some("[") => self.push("]", column),
            Some("(") => self.push(")", column),
            Some("}") => {
                while self.top().kind == "statement" {
                    self.pop();
                }
                if self.top().kind == "}" {
                    self.pop();
                }
                while self.top().kind == "statement" {
                    self.pop();
                }
            }
            Some(punc) if punc == ctx.kind => self.pop(),
            _ if matches!(ctx.kind, "}" | "top")
                || (ctx.kind == "statement" && punc == Some("newstatement")) =>
            {
                self.push("statement", column)
            }
            _ => (),
        }
        self.last = punc.or(style);
        style
    }

    pub(super) fn indent(&self, text_after: &str, unit: usize) -> Option<usize> {
        if !matches!(self.frames.last(), None | Some(Frame::Brace(_))) {
            return None;
        }
        let mut ctx = self.top();
        if ctx.kind == "statement" && !expect_expression(self.last, true) {
            ctx = self.contexts[self.contexts.len() - 2];
        }
        let first = text_after.chars().next();
        let closing = ctx.kind.len() == 1 && first == ctx.kind.chars().next();
        Some(match ctx.kind {
            "statement" => ctx
                .indented
                .saturating_add(if first == Some('{') { 0 } else { unit }),
            "top" => unit.saturating_sub(self.root_unit),
            _ if ctx.align == Some(true) => ctx.column.saturating_add(usize::from(!closing)),
            _ => ctx.indented.saturating_add(if closing { 0 } else { unit }),
        })
    }
}

fn word(ch: char) -> bool {
    ascii_word(ch) || ch == '$'
}

fn expect_expression(last: Option<&str>, newline: bool) -> bool {
    match last {
        None
        | Some(
            "operator" | "->" | "." | "[" | "{" | "(" | "," | ";" | ":" | "newstatement"
            | "keyword" | "proplabel",
        ) => true,
        Some("standalone") => !newline,
        _ => false,
    }
}
