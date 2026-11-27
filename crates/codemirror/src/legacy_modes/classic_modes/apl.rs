use super::{identifier, number_tail};
use crate::stream_parser::StringStream;

const FUNCTIONS: &str = "+−×÷⌈⌊∣⍳?⋆⍟○!⌹<≤=>≥≠≡≢∈⍷∪∩∼∨∧⍱⍲⍴,⍪⌽⊖⍉↑↓⊂⊃⌷⍋⍒⊤⊥⍕⍎⊣⊢";

pub(super) fn token(stream: &mut StringStream<'_>) -> Option<&'static str> {
    // The upstream flags are never used to select a token style or range.
    if stream.eat_space() {
        return None;
    }
    let ch = stream.next()?;
    Some(match ch {
        '"' | '\'' => {
            stream.eat_while(|next| next != ch);
            stream.next();
            "string"
        }
        '[' | '{' | '(' | ']' | '}' | ')' => return None,
        '⍬' => "atom",
        '¯' | '0'..='9' => {
            stream.eat_while(number_tail);
            "number"
        }
        '.' | '/' | '⌿' | '⍀' | '¨' | '⍣' | '←' => "operator",
        ch if FUNCTIONS.contains(ch) => "variableName.function.standard",
        '⍝' | '#' => {
            stream.skip_to_end();
            "comment"
        }
        '∘' if stream.eat('.').is_some() => "variableName.function",
        _ => {
            stream.eat_while(identifier);
            "keyword"
        }
    })
}
