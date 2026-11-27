use std::{borrow::Cow, collections::HashMap, sync::LazyLock};

use regex::Regex;

use super::{StringStream, word};

mod words;

#[derive(Clone, Copy)]
enum Context {
    Data,
    Proc,
    Macro,
    DataOrProc,
    All,
}

static WORDS: LazyLock<HashMap<&str, (&str, Context)>> = LazyLock::new(|| {
    words::DEFINITIONS
        .iter()
        .flat_map(|&(style, text, context)| {
            text.split_ascii_whitespace()
                .map(move |word| (word, (style, context)))
        })
        .collect()
});

static LEADING_DOT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[0-9]+(?:[eE][+-]?[0-9]+)?").expect("valid SAS leading dot pattern")
});
static LEADING_ZERO: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:[xX][0-9a-fA-F]+|0[0-7]+)").expect("valid SAS leading zero pattern")
});
static DECIMAL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[0-9]*\.?[0-9]*(?:[eE][+-]?[0-9]+)?").expect("valid SAS decimal pattern")
});

#[derive(Clone, Debug, Default)]
pub(super) struct State {
    data_step: bool,
    procedure: bool,
    macro_body: bool,
    next_word: bool,
    quote: Option<char>,
    comment: bool,
}

impl State {
    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        if stream.eat_space() {
            return None;
        }
        let ch = stream.next()?;
        if ch == '/' && stream.eat('*').is_some() {
            self.comment = true;
            return Some("comment");
        }
        if self.comment {
            if ch == '*' && stream.eat('/').is_some() {
                self.comment = false;
            } else if stream.skip_to("*") {
                stream.next();
                self.comment = stream.eat('/').is_none();
            } else {
                stream.skip_to_end();
            }
            return Some("comment");
        }
        if ch == '*' && stream.column() == stream.indentation() {
            stream.skip_to_end();
            return Some("comment");
        }
        if let Some(quote) = self.quote {
            if ch == quote
                || stream.skip_to(quote.encode_utf8(&mut [0; 4])) && stream.eat(quote).is_some()
            {
                self.quote = None;
            } else {
                stream.skip_to_end();
            }
            return Some("string");
        }
        if matches!(ch, '"' | '\'') {
            self.quote = Some(ch);
            return Some("string");
        }
        if ch.is_ascii_digit() || ch == '.' {
            let pattern = match ch {
                '.' => &*LEADING_DOT,
                '0' => &*LEADING_ZERO,
                _ => &*DECIMAL,
            };
            stream.match_regex(pattern, true);
            return Some("number");
        }
        if matches!(
            (ch, stream.peek()),
            ('<' | '>' | '!', Some('=')) | ('<', Some('>'))
        ) {
            stream.next();
            return Some("operator");
        }
        let after_first = stream.pos();
        if matches!(
            (ch, stream.peek()),
            ('g' | 'l' | 'n', Some('e'))
                | ('e', Some('q'))
                | ('i', Some('n'))
                | ('l' | 'g', Some('t'))
                | ('o', Some('r'))
        ) {
            stream.next();
            if stream.peek() == Some(' ') {
                return Some("operator");
            }
        } else if "=(:),{}.*<>+-/^[]".contains(ch) {
            return Some("operator");
        }

        let tail_start = stream.pos();
        let has_tail = stream.eat_while(|ch| word(ch) || matches!(ch, '%' | '&' | ';'));
        let word_end = stream.pos();
        let text = if tail_start == after_first {
            Cow::Borrowed(stream.current())
        } else {
            Cow::Owned(format!("{ch}{}", &stream.line()[tail_start..word_end]))
        };
        if has_tail && text.contains('&') {
            return Some("variable");
        }
        if self.next_word {
            stream.eat_while(word);
            if stream.peek() == Some('.') {
                stream.skip_to(" ");
            }
            self.next_word = false;
            return Some("variableName.special");
        }

        let text = text.to_lowercase();
        let definition = WORDS.get(text.as_str()).copied();
        if self.data_step {
            if text == "run;" {
                self.data_step = false;
                return Some("builtin");
            }
            // The extra read is also made when the next character is not a period.
            if stream.next() == Some('.') {
                return Some(if stream.peek().is_none_or(word) {
                    "variableName.special"
                } else {
                    "variable"
                });
            }
            if let Some((style, Context::Data | Context::DataOrProc | Context::All)) = definition {
                let end = if tail_start == after_first {
                    word_end
                } else {
                    // The keyword length omits the second character of an operator word.
                    stream.start() + text.len()
                };
                stream.set_pos(end).unwrap();
                return Some(style);
            }
        }
        if self.procedure {
            if matches!(text.as_str(), "run;" | "quit;") {
                self.procedure = false;
                return Some("builtin");
            }
            if let Some((style, Context::Proc | Context::DataOrProc | Context::All)) = definition {
                stream.eat_while(word);
                return Some(style);
            }
        }
        if self.macro_body {
            if text == "%mend" {
                stream.eat(';');
                self.macro_body = false;
                return Some("builtin");
            }
            if let Some((style, Context::Macro | Context::All)) = definition {
                stream.eat_while(word);
                return Some(style);
            }
            return Some("atom");
        }

        let (style, context) = definition?;
        let last = stream
            .current()
            .chars()
            .next_back()
            .expect("non-empty token");
        stream.back_up(last.len_utf8()).unwrap();
        stream.eat_while(word);
        match text.as_str() {
            "data" if stream.peek() != Some('=') => self.data_step = true,
            "proc" => self.procedure = true,
            "%macro" => self.macro_body = true,
            "footnote" => {
                stream.eat_if(|ch| matches!(ch, '1'..='9'));
                return Some("def");
            }
            _ => return matches!(context, Context::All).then_some(style),
        }
        self.next_word = true;
        Some("builtin")
    }
}
