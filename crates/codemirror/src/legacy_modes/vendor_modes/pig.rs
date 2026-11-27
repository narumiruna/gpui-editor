use std::sync::LazyLock;

use super::{
    ScanMode, StringStream, close_comment, identifier, listed, number_tail, string_continues,
};

const BUILTINS: &str = concat!(
    "ABS ACOS ARITY ASIN ATAN AVG BAGSIZE BINSTORAGE BLOOM BUILDBLOOM CBRT CEIL ",
    "CONCAT COR COS COSH COUNT COUNT_STAR COV CONSTANTSIZE CUBEDIMENSIONS DIFF DISTINCT DOUBLEABS ",
    "DOUBLEAVG DOUBLEBASE DOUBLEMAX DOUBLEMIN DOUBLEROUND DOUBLESUM EXP FLOOR FLOATABS FLOATAVG ",
    "FLOATMAX FLOATMIN FLOATROUND FLOATSUM GENERICINVOKER INDEXOF INTABS INTAVG INTMAX INTMIN ",
    "INTSUM INVOKEFORDOUBLE INVOKEFORFLOAT INVOKEFORINT INVOKEFORLONG INVOKEFORSTRING INVOKER ",
    "ISEMPTY JSONLOADER JSONMETADATA JSONSTORAGE LAST_INDEX_OF LCFIRST LOG LOG10 LOWER LONGABS ",
    "LONGAVG LONGMAX LONGMIN LONGSUM MAX MIN MAPSIZE MONITOREDUDF NONDETERMINISTIC OUTPUTSCHEMA  ",
    "PIGSTORAGE PIGSTREAMING RANDOM REGEX_EXTRACT REGEX_EXTRACT_ALL REPLACE ROUND SIN SINH SIZE ",
    "SQRT STRSPLIT SUBSTRING SUM STRINGCONCAT STRINGMAX STRINGMIN STRINGSIZE TAN TANH TOBAG ",
    "TOKENIZE TOMAP TOP TOTUPLE TRIM TEXTLOADER TUPLESIZE UCFIRST UPPER UTF8STORAGECONVERTER ",
);

const KEYWORDS: &str = concat!(
    "VOID IMPORT RETURNS DEFINE LOAD FILTER FOREACH ORDER CUBE DISTINCT COGROUP ",
    "JOIN CROSS UNION SPLIT INTO IF OTHERWISE ALL AS BY USING INNER OUTER ONSCHEMA PARALLEL ",
    "PARTITION GROUP AND OR NOT GENERATE FLATTEN ASC DESC IS STREAM THROUGH STORE MAPREDUCE ",
    "SHIP CACHE INPUT OUTPUT STDERROR STDIN STDOUT LIMIT SAMPLE LEFT RIGHT FULL EQ GT LT GTE LTE ",
    "NEQ MATCHES TRUE FALSE DUMP",
);

const TYPES: &str = "BOOLEAN INT LONG FLOAT DOUBLE CHARARRAY BYTEARRAY BAG TUPLE MAP ";

pub(super) fn autocomplete() -> &'static [&'static str] {
    static WORDS: LazyLock<Vec<&str>> = LazyLock::new(|| {
        BUILTINS
            .trim_end_matches(' ')
            .split(' ')
            .chain(TYPES.trim_end_matches(' ').split(' '))
            .chain(KEYWORDS.split(' '))
            .collect()
    });
    WORDS.as_slice()
}

#[derive(Clone, Debug, Default)]
pub(super) struct State(ScanMode);

impl State {
    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        if stream.eat_space() {
            return None;
        }
        match self.0 {
            ScanMode::Base => self.base(stream),
            ScanMode::Comment => Some(self.comment(stream)),
            ScanMode::String(quote) => Some(self.string(stream, quote)),
        }
    }

    fn base(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        let ch = stream.next()?;
        Some(match ch {
            '"' | '\'' => self.string(stream, ch),
            '[' | ']' | '{' | '}' | '(' | ')' | ',' | ';' | '.' => return None,
            '0'..='9' => {
                stream.eat_while(number_tail);
                "number"
            }
            '/' if stream.eat('*').is_some() => self.comment(stream),
            '-' if stream.eat('-').is_some() => {
                stream.skip_to_end();
                "comment"
            }
            ch if operator(ch) => {
                stream.eat_while(operator);
                "operator"
            }
            _ => {
                stream.eat_while(identifier);
                let current = stream.current().to_uppercase();
                if listed(KEYWORDS, &current) {
                    // The legacy token consumes a single closing parenthesis or dot.
                    if stream.eat(')').is_some() || stream.eat('.').is_some() {
                        "variable"
                    } else {
                        "keyword"
                    }
                } else if listed(BUILTINS, &current) {
                    "builtin"
                } else if listed(TYPES, &current) {
                    "type"
                } else {
                    "variable"
                }
            }
        })
    }

    fn comment(&mut self, stream: &mut StringStream<'_>) -> &'static str {
        self.0 = if close_comment(stream, false) {
            ScanMode::Base
        } else {
            ScanMode::Comment
        };
        "comment"
    }

    fn string(&mut self, stream: &mut StringStream<'_>, quote: char) -> &'static str {
        self.0 = if string_continues(stream, quote) {
            ScanMode::String(quote)
        } else {
            ScanMode::Base
        };
        "error"
    }
}

fn operator(ch: char) -> bool {
    "*+-%<>=&?:/!|".contains(ch)
}
