use super::{js_space, quote_continues};
use crate::stream_parser::StringStream;

#[derive(Clone, Debug, Default)]
pub(super) struct State {
    quoted: bool,
}

impl State {
    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        if stream.eat_space() {
            return None;
        }
        if self.quoted {
            self.quoted = quote_continues(stream);
            return Some("string");
        }

        Some(match stream.next()? {
            '"' => {
                self.quoted = quote_continues(stream);
                "string"
            }
            ch if operator(ch) => {
                if matches!(ch, '|' | '&') {
                    stream.eat(ch);
                }
                "operator"
            }
            ch if string_char(ch) => {
                stream.eat_while(string_char);
                let word = stream.current();
                if matches!(word, "OR" | "AND" | "NOT" | "TO") {
                    "operator"
                } else if number(word) {
                    "number"
                } else if stream.peek() == Some(':') {
                    "propertyName"
                } else {
                    "string"
                }
            }
            _ => return None,
        })
    }
}

fn operator(ch: char) -> bool {
    "|!+-*?~^&".contains(ch)
}

fn string_char(ch: char) -> bool {
    !js_space(ch) && !operator(ch) && !":()[]{}\"\\".contains(ch)
}

fn number(word: &str) -> bool {
    // An exponent sign is a separate token. Only decimal spellings and the
    // special values can equal the result of JavaScript's parseFloat().toString().
    if matches!(word, "NaN" | "Infinity") {
        return true;
    }
    let Ok(value) = word.parse::<f64>() else {
        return false;
    };
    if value == 0.0 {
        return word == "0";
    }
    if !(1e-6..1e21).contains(&value) {
        return false;
    }

    // Unlike f64's Display, the JSON formatter rounds shortest-decimal ties
    // to even, as JavaScript does. Convert its exponent notation to decimal.
    let shortest = serde_json::Number::from_f64(value)
        .expect("finite number")
        .to_string();
    let Some((mantissa, exponent)) = shortest.split_once('e') else {
        return shortest.strip_suffix(".0").unwrap_or(&shortest) == word;
    };
    let integer_len = mantissa.find('.').unwrap_or(mantissa.len());
    let exponent: isize = exponent.parse().expect("decimal exponent");
    let point = integer_len as isize + exponent;
    let mut digits = mantissa.replace('.', "");
    if point <= 0 {
        return word
            .strip_prefix("0.")
            .and_then(|tail| tail.strip_suffix(digits.as_str()))
            .is_some_and(|zeros| {
                zeros.len() == point.unsigned_abs() && zeros.bytes().all(|ch| ch == b'0')
            });
    }
    let point = point as usize;
    if point < digits.len() {
        digits.insert(point, '.');
    } else {
        digits.extend(std::iter::repeat_n('0', point - digits.len()));
    }
    digits == word
}
