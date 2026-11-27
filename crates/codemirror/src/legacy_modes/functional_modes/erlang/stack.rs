use super::{BLOCK_START, CLAUSE_BLOCK, CLOSE, Kind, OPEN, StringStream};

#[derive(Clone, Debug, Default)]
pub(super) struct Stack(Vec<Token>);

#[derive(Clone, Debug)]
struct Token {
    text: Box<str>,
    column: usize,
    indent: usize,
    kind: Kind,
}

#[derive(Clone, Copy)]
enum Reduce {
    Between,
    Pair,
    Group,
    KeepTop,
    KeepMatch,
    Clause,
}

impl Stack {
    pub(super) fn peek(&self, depth: usize) -> Option<&str> {
        self.0.iter().rev().nth(depth - 1).map(|token| &*token.text)
    }

    pub(super) fn push(&mut self, kind: Kind, stream: &StringStream<'_>) {
        if matches!(kind, Kind::Comment | Kind::Space) {
            return;
        }
        let stack = &mut self.0;
        if stack.len() > 1
            && stack.last().is_some_and(|token| token.kind == Kind::Record)
            && kind == Kind::Dot
        {
            stack.pop();
        } else {
            if stack.len() > 1 && stack.last().is_some_and(|token| token.kind == Kind::Group) {
                stack.pop();
            }
            stack.push(Token {
                text: stream.current().into(),
                column: stream.column(),
                indent: stream.indentation(),
                kind,
            });
        }
        self.reduce();
    }

    fn reduce(&mut self) {
        let stack = &mut self.0;
        let Some((top, rest)) = stack.split_last() else {
            return;
        };
        if top.kind == Kind::Dot {
            stack.clear();
            return;
        }
        if rest.len() > 1
            && top.kind == Kind::Fun
            && rest.last().is_some_and(|prev| &*prev.text == "fun")
        {
            stack.truncate(rest.len() - 1);
            return;
        }
        let (rule, words): (Reduce, &[&str]) = match &*top.text {
            "}" => (Reduce::Group, &["{"]),
            "]" => (Reduce::Pair, &["["]),
            ")" => (Reduce::Pair, &["("]),
            ">>" => (Reduce::Pair, &["<<"]),
            "end" => (Reduce::Pair, BLOCK_START),
            "," => (
                Reduce::KeepMatch,
                &["begin", "try", "when", "->", ",", "(", "[", "{", "<<"],
            ),
            "->" => {
                if let Some(index) = rest.iter().rposition(|token| &*token.text == "when") {
                    self.apply(index, Reduce::KeepTop);
                    return;
                }
                (Reduce::Between, CLAUSE_BLOCK)
            }
            ";" => (
                Reduce::Clause,
                &["case", "fun", "if", "receive", "try", "when"],
            ),
            "catch" => (Reduce::KeepMatch, &["try"]),
            "of" => (Reduce::KeepMatch, &["case"]),
            "after" => (Reduce::KeepMatch, &["receive", "try"]),
            _ => return,
        };
        if let Some(index) = rest.iter().rposition(|token| words.contains(&&*token.text)) {
            self.apply(index, rule);
        } else if matches!(rule, Reduce::Clause) {
            stack.clear();
        }
    }

    fn apply(&mut self, index: usize, rule: Reduce) {
        let stack = &mut self.0;
        match rule {
            Reduce::Between | Reduce::KeepTop => {
                let top = stack.pop().expect("reduction requires a top token");
                stack.truncate(index + usize::from(matches!(rule, Reduce::Between)));
                stack.push(top);
            }
            Reduce::Pair => stack.truncate(index),
            Reduce::Group => {
                stack.truncate(index);
                stack.push(Token {
                    text: "group".into(),
                    column: 0,
                    indent: 0,
                    kind: Kind::Group,
                });
            }
            Reduce::KeepMatch | Reduce::Clause => stack.truncate(index + 1),
        }
    }

    pub(super) fn indent(&self, text_after: &str, unit: usize) -> Option<usize> {
        let [.., prev, curr] = self.0.as_slice() else {
            return Some(0);
        };
        let next = first_word(text_after);
        let curr_text = &*curr.text;
        let find = |words: &[&str]| {
            self.0
                .iter()
                .rev()
                .find(|token| words.contains(&&*token.text))
        };
        let add = |column: usize| column.saturating_add(unit);
        Some(if curr_text == "when" {
            add(curr.column)
        } else if next == "when" && prev.kind == Kind::Function {
            add(prev.indent)
        } else if next == "(" && curr_text == "fun" {
            curr.column.saturating_add(3)
        } else if next == "catch"
            && let Some(token) = find(&["try"])
        {
            token.column
        } else if ["end", "after", "of"].contains(&next) {
            find(BLOCK_START)?.column
        } else if CLOSE.contains(&next) {
            find(OPEN)?.column
        } else if [",", "|", "||"]
            .iter()
            .any(|&word| word == curr_text || word == next)
        {
            self.0[..self.0.len() - 1]
                .iter()
                .rev()
                .find(|token| token.kind == Kind::Open)
                .map_or(unit, |token| token.column.saturating_add(token.text.len()))
        } else if curr_text == "->" {
            let column = add(prev.column);
            if CLAUSE_BLOCK.contains(&&*prev.text) {
                add(column)
            } else {
                column
            }
        } else if OPEN.contains(&curr_text) {
            curr.column.saturating_add(curr.text.len())
        } else {
            self.default_anchor().map_or(0, |token| add(token.column))
        })
    }

    fn default_anchor(&self) -> Option<&Token> {
        let stop = self.0.iter().rposition(|token| {
            matches!(token.kind, Kind::Open | Kind::Separator | Kind::Keyword)
        })?;
        let operator_after_stop = self.0[stop + 1..]
            .iter()
            .any(|token| token.kind == Kind::Operator);
        Some(&self.0[stop + usize::from(operator_after_stop)])
    }
}

fn first_word(text: &str) -> &str {
    if text.starts_with(">>") {
        &text[..2]
    } else if text.starts_with([',', '}', ']', ')', '(']) {
        &text[..1]
    } else if text.starts_with('|') {
        &text[..text.find(|ch| ch != '|').unwrap_or(text.len())]
    } else {
        &text[..text
            .find(|ch: char| !ch.is_ascii_lowercase())
            .unwrap_or(text.len())]
    }
}
