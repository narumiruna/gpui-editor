use super::{super::js_line_terminator, word};
use crate::stream_parser::StringStream;

const FUNCTIONS: &str = "abs acos allShortestPaths asin atan atan2 avg ceil coalesce collect cos cot count degrees e endnode exp extract filter floor haversin head id keys labels last left length log log10 lower ltrim max min node nodes percentileCont percentileDisc pi radians rand range reduce rel relationship relationships replace reverse right round rtrim shortestPath sign sin size split sqrt startnode stdev stdevp str substring sum tail tan timestamp toFloat toInt toString trim type upper";
const PREDICATES: &str = "all and any contains exists has in none not or single xor";
const KEYWORDS: &str = "as asc ascending assert by case commit constraint create csv cypher delete desc descending detach distinct drop else end ends explain false fieldterminator foreach from headers in index is join limit load match merge null on optional order periodic profile remove return scan set skip start starts then true union unique unwind using when where with call yield";
const SYSTEM_KEYWORDS: &str = "access active assign all alter as catalog change copy create constraint constraints current database databases dbms default deny drop element elements exists from grant graph graphs if index indexes label labels management match name names new node nodes not of on or password populated privileges property read relationship relationships remove replace required revoke role roles set show start status stop suspended to traverse type types user users with write";

#[derive(Clone, Debug, Default)]
pub(super) struct State {
    contexts: Vec<Context>,
    indent: usize,
}

#[derive(Clone, Debug)]
struct Context {
    // None marks a pattern context, which has no closing delimiter.
    closing: Option<char>,
    align: Option<bool>,
    indent: usize,
    column: usize,
}

impl State {
    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        if stream.sol() {
            if let Some(context) = self.contexts.last_mut() {
                context.align.get_or_insert(false);
            }
            self.indent = stream.indentation();
        }
        if stream.eat_space() {
            return None;
        }
        let (style, punctuation) = scan(stream)?;
        if let Some(context) = self.contexts.last_mut()
            && style != Some("comment")
            && context.closing.is_some()
        {
            context.align.get_or_insert(true);
        }

        match punctuation {
            Some('(') => self.push(Some(')'), stream.column()),
            Some('[') => self.push(Some(']'), stream.column()),
            Some('{') => self.push(Some('}'), stream.column()),
            Some(')' | ']' | '}') => {
                while self.in_pattern() {
                    self.pop();
                }
                if self.contexts.last().and_then(|context| context.closing) == punctuation {
                    self.pop();
                }
            }
            Some('.') if self.in_pattern() => self.pop(),
            _ if matches!(style, Some("atom" | "string" | "variable")) => {
                if let Some(context) = self.contexts.last_mut() {
                    match context.closing {
                        Some(']' | '}') => self.push(None, stream.column()),
                        None if context.align != Some(true) => {
                            context.align = Some(true);
                            context.column = stream.column();
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
        style
    }

    pub(super) fn indent(&self, text_after: &str, unit: usize) -> usize {
        let first = text_after.chars().next();
        let context = if matches!(first, Some(']' | '}')) {
            self.contexts
                .iter()
                .rev()
                .find(|context| context.closing.is_some())
        } else {
            self.contexts.last()
        };
        let Some(context) = context else { return 0 };
        let closing = first.is_some() && first == context.closing;
        if context.align == Some(true) {
            context.column + usize::from(!closing)
        } else {
            context
                .indent
                .saturating_add(if closing { 0 } else { unit })
        }
    }

    fn in_pattern(&self) -> bool {
        self.contexts
            .last()
            .is_some_and(|context| context.closing.is_none())
    }

    fn push(&mut self, closing: Option<char>, column: usize) {
        self.contexts.push(Context {
            closing,
            align: None,
            indent: self.indent,
            column,
        });
    }

    fn pop(&mut self) {
        if let Some(context) = self.contexts.pop() {
            self.indent = context.indent;
        }
    }
}

fn scan(stream: &mut StringStream<'_>) -> Option<(Option<&'static str>, Option<char>)> {
    let ch = stream.next()?;
    let style = match ch {
        '"' | '\'' => {
            let rest = &stream.line()[stream.pos()..];
            if let Some(end) = rest.find(|c| c == ch || js_line_terminator(c))
                && rest[end..].starts_with(ch)
            {
                stream
                    .set_pos(stream.pos() + end + 1)
                    .expect("quote is a scalar boundary");
            }
            "string"
        }
        '{' | '}' | '(' | ')' | ',' | '.' | ';' | '[' | ']' => {
            return Some((Some("punctuation"), Some(ch)));
        }
        '/' if stream.eat('/').is_some() => {
            stream.skip_to_end();
            "comment"
        }
        c if operator(c) => {
            stream.eat_while(operator);
            return Some((None, None));
        }
        _ => {
            stream.eat_while(word);
            if stream.eat(':').is_some() {
                stream.eat_while(|ch| ch == '-' || word(ch));
                "atom"
            } else {
                let text = stream.current();
                if contains(FUNCTIONS, text) {
                    "builtin"
                } else if contains(PREDICATES, text) {
                    "def"
                } else if contains(KEYWORDS, text) || contains(SYSTEM_KEYWORDS, text) {
                    "keyword"
                } else {
                    "variable"
                }
            }
        }
    };
    Some((Some(style), None))
}

fn operator(ch: char) -> bool {
    "*+-<>=&|~%^".contains(ch)
}

fn contains(words: &str, text: &str) -> bool {
    words
        .split_ascii_whitespace()
        .any(|word| word.eq_ignore_ascii_case(text))
}
