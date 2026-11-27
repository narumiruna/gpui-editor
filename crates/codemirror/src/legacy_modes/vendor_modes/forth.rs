use std::collections::HashSet;

use super::{StringStream, js_space, listed};

const CORE: &str = "INVERT AND OR XOR
2* 2/ LSHIFT RSHIFT
0= = 0< < > U< MIN MAX
2DROP 2DUP 2OVER 2SWAP ?DUP DEPTH DROP DUP OVER ROT SWAP
>R R> R@
+ - 1+ 1- ABS NEGATE
S>D * M* UM*
FM/MOD SM/REM UM/MOD */ */MOD / /MOD MOD
HERE , @ ! CELL+ CELLS C, C@ C! CHARS 2@ 2!
ALIGN ALIGNED +! ALLOT
CHAR [CHAR] [ ] BL
FIND EXECUTE IMMEDIATE COUNT LITERAL STATE
; DOES> >BODY
EVALUATE
SOURCE >IN
<# # #S #> HOLD SIGN BASE >NUMBER HEX DECIMAL
FILL MOVE
. CR EMIT SPACE SPACES TYPE U. .R U.R
ACCEPT
TRUE FALSE
<> U> 0<> 0>
NIP TUCK ROLL PICK
2>R 2R@ 2R>
WITHIN UNUSED MARKER
I J
TO
COMPILE, [COMPILE]
SAVE-INPUT RESTORE-INPUT
PAD ERASE
2LITERAL DNEGATE
D- D+ D0< D0= D2* D2/ D< D= DMAX DMIN D>S DABS
M+ M*/ D. D.R 2ROT DU<
CATCH THROW
FREE RESIZE ALLOCATE
CS-PICK CS-ROLL
GET-CURRENT SET-CURRENT FORTH-WORDLIST GET-ORDER SET-ORDER
PREVIOUS SEARCH-WORDLIST WORDLIST FIND ALSO ONLY FORTH DEFINITIONS ORDER
-TRAILING /STRING SEARCH COMPARE CMOVE CMOVE> BLANK SLITERAL";
const IMMEDIATE: &str = "IF ELSE THEN BEGIN WHILE REPEAT UNTIL RECURSE [IF] [ELSE] [THEN] ?DO DO LOOP +LOOP UNLOOP LEAVE EXIT AGAIN CASE OF ENDOF ENDCASE";
const DEFINERS: &str = "VARIABLE 2VARIABLE CONSTANT 2CONSTANT CREATE POSTPONE VALUE WORD";

#[derive(Clone, Debug, Default)]
pub(super) struct State {
    compiling: bool,
    definitions: HashSet<String>,
}

impl State {
    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        if stream.eat_space() || stream.eol() {
            return None;
        }
        let start = stream.pos();
        let remaining = &stream.line()[start..];
        let (word, through_space) = split_word(remaining);
        let uppercase = word.to_uppercase();
        let rest = &remaining[through_space..];

        if self.compiling {
            if matches!(word, ";" | "[") {
                self.compiling = false;
                stream.set_pos(start + word.len()).unwrap();
                return Some("builtin");
            }
            if word == "POSTPONE" && !rest.is_empty() {
                stream
                    .set_pos(start + through_space + split_word(rest).1)
                    .unwrap();
                return Some("builtin");
            }
        } else {
            if word == "]" || word.eq_ignore_ascii_case(":NONAME") {
                self.compiling = true;
                let whitespace = remaining[word.len()..]
                    .chars()
                    .next()
                    .map_or(0, char::len_utf8);
                stream.set_pos(start + word.len() + whitespace).unwrap();
                return Some("builtin");
            }
            if !rest.is_empty() {
                let (name, end) = split_word(rest);
                if word == ":" || (word.is_ascii() && listed(DEFINERS, &uppercase)) {
                    self.definitions.insert(name.to_uppercase());
                    self.compiling = word == ":";
                    stream.set_pos(start + through_space + end).unwrap();
                    return Some("def");
                }
                if matches!(word, "'" | "[']") {
                    stream.set_pos(start + through_space + end).unwrap();
                    return Some("builtin");
                }
            }
        }

        stream.set_pos(start + through_space).unwrap();
        Some(if self.definitions.contains(&uppercase) {
            "variable"
        } else if word == "\\" {
            stream.skip_to_end();
            "comment"
        } else if listed(CORE, &uppercase) {
            "builtin"
        } else if listed(IMMEDIATE, &uppercase) {
            "keyword"
        } else if word == "(" {
            take_until(stream, ')');
            "comment"
        } else if word == ".(" {
            take_until(stream, ')');
            "string"
        } else if matches!(word, "S\"" | ".\"" | "C\"") {
            take_until(stream, '"');
            "string"
        } else if number(word).is_some_and(|value| value != 68_719_476_735.0) {
            "number"
        } else {
            "atom"
        })
    }
}

fn split_word(input: &str) -> (&str, usize) {
    let end = input.find(js_space).unwrap_or(input.len());
    let rest = input[end..].trim_start_matches(js_space);
    (&input[..end], input.len() - rest.len())
}

fn take_until(stream: &mut StringStream<'_>, end: char) {
    stream.eat_while(|ch| ch != end);
    stream.eat(end);
}

// The mode classifies a word with JS subtraction, not with the Forth base.
fn number(word: &str) -> Option<f64> {
    if let Some(rest) = word.strip_prefix('0') {
        let radix = match rest.as_bytes().first() {
            Some(b'x' | b'X') => 16,
            Some(b'o' | b'O') => 8,
            Some(b'b' | b'B') => 2,
            _ => 0,
        };
        if radix != 0 {
            let digits = &rest[1..];
            return (!digits.is_empty())
                .then(|| {
                    digits.chars().try_fold(0.0, |value, ch| {
                        Some(value * f64::from(radix) + f64::from(ch.to_digit(radix)?))
                    })
                })
                .flatten();
        }
    }
    let magnitude = word.trim_start_matches(['+', '-']);
    if magnitude == "Infinity" || magnitude.starts_with(|ch: char| ch.is_ascii_digit() || ch == '.')
    {
        word.parse().ok()
    } else {
        None
    }
}
