use std::{collections::HashMap, sync::LazyLock};

use super::ascii_word;
use crate::stream_parser::StringStream;

#[derive(Clone, Debug, Default)]
pub(super) enum State {
    #[default]
    Normal,
    Block {
        depth: usize,
        style: &'static str,
    },
    StringGap,
}

impl State {
    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        let style = match *self {
            Self::Normal => self.normal(stream),
            Self::Block { depth, style } => Some(self.block(stream, depth, style)),
            Self::StringGap => Some(if stream.eat('\\').is_some() {
                self.string(stream)
            } else {
                stream.next();
                *self = Self::Normal;
                "error"
            }),
        };
        WORDS.get(stream.current()).copied().or(style)
    }

    fn normal(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        if stream.eat_while(is_space) {
            return None;
        }
        let style = match stream.next()? {
            '{' if stream.eat('-').is_some() => {
                let style = if stream.eat('#').is_some() {
                    "meta"
                } else {
                    "comment"
                };
                self.block(stream, 1, style)
            }
            '(' | ')' | ',' | ';' | '[' | ']' | '`' | '{' | '}' => return None,
            '\'' => {
                stream.eat('\\');
                stream.next();
                if stream.eat('\'').is_some() {
                    "string"
                } else {
                    "error"
                }
            }
            '"' => self.string(stream),
            'A'..='Z' => {
                stream.eat_while(is_identifier);
                if stream.eat('.').is_some() {
                    "qualifier"
                } else {
                    "type"
                }
            }
            'a'..='z' | '_' => {
                stream.eat_while(is_identifier);
                "variable"
            }
            ch if ch.is_ascii_digit() => number(stream, ch),
            '.' if stream.eat('.').is_some() => "keyword",
            ch if is_symbol(ch) => {
                if ch == '-' && stream.eat('-').is_some() {
                    stream.eat_while(|ch| ch == '-');
                    if stream.eat_if(is_symbol).is_none() {
                        stream.skip_to_end();
                        return Some("comment");
                    }
                }
                stream.eat_while(is_symbol);
                "variable"
            }
            _ => "error",
        };
        Some(style)
    }

    fn block(
        &mut self,
        stream: &mut StringStream<'_>,
        mut depth: usize,
        style: &'static str,
    ) -> &'static str {
        while let Some(ch) = stream.next() {
            if ch == '{' && stream.eat('-').is_some() {
                depth += 1;
            } else if ch == '-' && stream.eat('}').is_some() {
                depth -= 1;
                if depth == 0 {
                    *self = Self::Normal;
                    return style;
                }
            }
        }
        *self = Self::Block { depth, style };
        style
    }

    fn string(&mut self, stream: &mut StringStream<'_>) -> &'static str {
        while let Some(ch) = stream.next() {
            if ch == '"' {
                *self = Self::Normal;
                return "string";
            }
            if ch == '\\' {
                if stream.eol() || stream.eat_if(is_space).is_some() {
                    *self = Self::StringGap;
                    return "string";
                }
                stream.next();
            }
        }
        *self = Self::Normal;
        "error"
    }
}

fn number(stream: &mut StringStream<'_>, first: char) -> &'static str {
    if first == '0' {
        if stream.eat_if(|ch| matches!(ch, 'x' | 'X')).is_some() {
            stream.eat_while(|ch| ch.is_ascii_hexdigit());
            return "integer";
        }
        if stream.eat_if(|ch| matches!(ch, 'o' | 'O')).is_some() {
            stream.eat_while(|ch| matches!(ch, '0'..='7'));
            return "number";
        }
    }
    stream.eat_while(|ch| ch.is_ascii_digit());
    let fraction = super::remaining(stream)
        .strip_prefix('.')
        .is_some_and(|s| s.starts_with(|ch: char| ch.is_ascii_digit()));
    if fraction {
        stream.next();
        stream.eat_while(|ch| ch.is_ascii_digit());
    }
    if stream.eat_if(|ch| matches!(ch, 'e' | 'E')).is_some() {
        stream.eat_if(|ch| matches!(ch, '-' | '+'));
        stream.eat_while(|ch| ch.is_ascii_digit());
    }
    "number"
}

fn is_identifier(ch: char) -> bool {
    ascii_word(ch) || ch == '\'' || ch >= '\u{a1}'
}

fn is_symbol(ch: char) -> bool {
    "-!#$%&*+./<=>?@\\^|~:".contains(ch)
}

fn is_space(ch: char) -> bool {
    matches!(ch, ' ' | '\t' | '\u{b}' | '\u{c}')
}

// The legacy mode also applies this table to continued comments and strings.
static WORDS: LazyLock<HashMap<&str, &str>> = LazyLock::new(|| {
    [
        (
            "keyword",
            "case class data default deriving do else foreign if import in infix infixl infixr
             instance let module newtype of then type where _ .. : :: = \\ <- -> @ ~ =>",
        ),
        (
            "builtin",
            "!! $! $ && + ++ - . / /= < <* <= <$> <*> =<< == > >= >> >>= ^ ^^ || * *> **
             Applicative Bool Bounded Char Double EQ Either Enum Eq False FilePath Float Floating
             Fractional Functor GT IO IOError Int Integer Integral Just LT Left Maybe Monad Nothing
             Num Ord Ordering Rational Read ReadS Real RealFloat RealFrac Right Show ShowS String True
             abs acos acosh all and any appendFile asTypeOf asin asinh atan atan2 atanh break catch
             ceiling compare concat concatMap const cos cosh curry cycle decodeFloat div divMod drop
             dropWhile either elem encodeFloat enumFrom enumFromThen enumFromThenTo enumFromTo error
             even exp exponent fail filter flip floatDigits floatRadix floatRange floor fmap foldl
             foldl1 foldr foldr1 fromEnum fromInteger fromIntegral fromRational fst gcd getChar
             getContents getLine head id init interact ioError isDenormalized isIEEE isInfinite isNaN
             isNegativeZero iterate last lcm length lex lines log logBase lookup map mapM mapM_ max
             maxBound maximum maybe min minBound minimum mod negate not notElem null odd or otherwise
             pi pred print product properFraction pure putChar putStr putStrLn quot quotRem read
             readFile readIO readList readLn readParen reads readsPrec realToFrac recip rem repeat
             replicate return reverse round scaleFloat scanl scanl1 scanr scanr1 seq sequence
             sequence_ show showChar showList showParen showString shows showsPrec significand signum
             sin sinh snd span splitAt sqrt subtract succ sum tail take takeWhile tan tanh toEnum
             toInteger toRational truncate uncurry undefined unlines until unwords unzip unzip3
             userError words writeFile zip zip3 zipWith zipWith3",
        ),
    ]
    .into_iter()
    .flat_map(|(style, words)| words.split_ascii_whitespace().map(move |word| (word, style)))
    .collect()
});
