use std::sync::LazyLock;

use regex::Regex;

use super::{Context, StringStream, close_comment, identifier, listed};

const KEYWORDS: &str = "term method accu rule then is and or if default";
const START_BLOCKS: &str = "var_input var_output fuzzify defuzzify function_block ruleblock";
const END_BLOCKS: &str = "end_ruleblock end_defuzzify end_function_block end_fuzzify end_var";
const ATOMS: &str = "true false nan real min max cog cogs";

#[derive(Clone, Debug)]
pub(super) struct State {
    contexts: Vec<Context<()>>,
    initial_unit: usize,
    indented: usize,
    in_comment: bool,
}

impl State {
    pub(super) fn new(indent_unit: usize) -> Self {
        Self {
            contexts: vec![Context {
                align: Some(false),
                ..Context::new((), 0, 0)
            }],
            initial_unit: indent_unit,
            indented: 0,
            in_comment: false,
        }
    }

    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        let context = self.contexts.last_mut().unwrap();
        if stream.sol() {
            context.align.get_or_insert(false);
            self.indented = stream.indentation();
        }
        if stream.eat_space() {
            return None;
        }
        if self.in_comment {
            self.in_comment = !close_comment(stream, true);
            return Some("comment");
        }
        let ch = stream.next()?;
        if matches!(ch, '/' | '(') {
            if stream.eat('*').is_some() {
                self.in_comment = !close_comment(stream, true);
                return Some("comment");
            }
            if stream.eat('/').is_some() {
                stream.skip_to_end();
                return Some("comment");
            }
        }
        context.align.get_or_insert(true);
        if ch == '.' || ch.is_ascii_digit() {
            number(stream, ch);
            return Some("number");
        }
        if operator(ch) {
            stream.eat_while(operator);
            return Some("operator");
        }
        stream.eat_while(|ch| identifier(ch) || ch >= '\u{a1}');

        let current = stream.current().to_lowercase();
        if listed(START_BLOCKS, &current) {
            self.contexts
                .push(Context::new((), self.indented, stream.column()));
            Some("keyword")
        } else if listed(END_BLOCKS, &current) {
            if self.contexts.len() > 1 {
                self.indented = self.contexts.pop().unwrap().indented;
            }
            Some("keyword")
        } else if listed(KEYWORDS, &current) {
            Some("keyword")
        } else if listed(ATOMS, &current) {
            Some("atom")
        } else {
            Some("variable")
        }
    }

    pub(super) fn indent(&self, text_after: &str, unit: usize) -> usize {
        if self.in_comment {
            return 0;
        }
        let closing = listed(END_BLOCKS, text_after);
        if self.contexts.len() == 1 {
            return if closing {
                0
            } else {
                unit.saturating_sub(self.initial_unit)
            };
        }
        self.contexts.last().unwrap().indent(closing, unit)
    }
}

fn operator(ch: char) -> bool {
    "+-*&^%:=<>!|/".contains(ch)
}

fn number(stream: &mut StringStream<'_>, ch: char) {
    static FRACTION: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^[0-9]+(?:[eE][+-]?[0-9]+)?").unwrap());
    static RADIX: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^(?:[xX][0-9a-fA-F]+|0[0-7]+)").unwrap());
    static DECIMAL: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^[0-9]*\.?[0-9]*(?:[eE][+-]?[0-9]+)?").unwrap());
    let pattern = match ch {
        '.' => &FRACTION,
        '0' => &RADIX,
        _ => &DECIMAL,
    };
    stream.match_regex(pattern, true);
}
