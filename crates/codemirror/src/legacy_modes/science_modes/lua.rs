use std::sync::LazyLock;

use regex::Regex;

use super::{remaining, word};
use crate::{legacy_modes::js_space, stream_parser::StringStream};

const KEYWORDS: &str = "and break elseif false nil not or return true function end if then else do while repeat until for in local";
const BUILTINS: &str = "
_G _VERSION assert collectgarbage dofile error getfenv getmetatable ipairs load
loadfile loadstring module next pairs pcall print rawequal rawget rawset require
select setfenv setmetatable tonumber tostring type unpack xpcall
coroutine.create coroutine.resume coroutine.running coroutine.status coroutine.wrap coroutine.yield
debug.debug debug.getfenv debug.gethook debug.getinfo debug.getlocal debug.getmetatable
debug.getregistry debug.getupvalue debug.setfenv debug.sethook debug.setlocal debug.setmetatable
debug.setupvalue debug.traceback
close flush lines read seek setvbuf write
io.close io.flush io.input io.lines io.open io.output io.popen io.read io.stderr io.stdin
io.stdout io.tmpfile io.type io.write
math.abs math.acos math.asin math.atan math.atan2 math.ceil math.cos math.cosh math.deg
math.exp math.floor math.fmod math.frexp math.huge math.ldexp math.log math.log10 math.max
math.min math.modf math.pi math.pow math.rad math.random math.randomseed math.sin math.sinh
math.sqrt math.tan math.tanh
os.clock os.date os.difftime os.execute os.exit os.getenv os.remove os.rename os.setlocale
os.time os.tmpname
package.cpath package.loaded package.loaders package.loadlib package.path package.preload
package.seeall
string.byte string.char string.dump string.find string.format string.gmatch string.gsub
string.len string.lower string.match string.rep string.reverse string.sub string.upper
table.concat table.insert table.maxn table.remove table.sort";

pub(super) fn indent_on_input(line: &str) -> bool {
    matches!(
        line.trim_start_matches(js_space),
        "end" | "until" | "else" | ")" | "}"
    )
}

#[derive(Clone, Debug, Default)]
pub(super) struct State {
    scan: Scan,
    depth: isize,
}

#[derive(Clone, Copy, Debug, Default)]
enum Scan {
    #[default]
    Base,
    Quoted(char),
    Bracketed {
        level: usize,
        comment: bool,
    },
}

impl State {
    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        static BUILTINS_RE: LazyLock<Regex> = LazyLock::new(|| {
            // Upstream does not escape the dots in the standard-library names.
            let alternatives = BUILTINS
                .split_ascii_whitespace()
                .collect::<Vec<_>>()
                .join("|");
            Regex::new(&format!("(?i)^(?:{alternatives})$")).unwrap()
        });
        if stream.eat_space() {
            return None;
        }
        let style = match self.scan {
            Scan::Base => self.base(stream),
            Scan::Quoted(quote) => Some(self.quoted(stream, quote)),
            Scan::Bracketed { level, comment } => Some(self.bracketed(stream, level, comment)),
        };
        if matches!(style, Some("comment" | "string")) {
            return style;
        }
        let current = stream.current();
        if ["function", "if", "repeat", "do", "(", "{"]
            .iter()
            .any(|item| item.eq_ignore_ascii_case(current))
        {
            self.depth += 1;
        } else if ["end", "until", ")", "}"]
            .iter()
            .any(|item| item.eq_ignore_ascii_case(current))
        {
            self.depth -= 1;
        }
        if style != Some("variable") {
            return style;
        }
        Some(
            if KEYWORDS
                .split_ascii_whitespace()
                .any(|item| item.eq_ignore_ascii_case(current))
            {
                "keyword"
            } else if BUILTINS_RE.is_match(current) {
                "builtin"
            } else {
                "variable"
            },
        )
    }

    pub(super) fn indent(&self, text_after: &str, unit: usize) -> usize {
        let closing = ["end", "until", ")", "}", "else"].iter().any(|prefix| {
            text_after
                .get(..prefix.len())
                .is_some_and(|first| first.eq_ignore_ascii_case(prefix))
        });
        (self.depth - isize::from(closing))
            .max(0)
            .unsigned_abs()
            .saturating_mul(unit)
    }

    fn base(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        match stream.next()? {
            '-' if stream.eat('-').is_some() => {
                if stream.eat('[').is_some() && matches!(stream.peek(), Some('[' | '=')) {
                    return Some(self.open_bracket(stream, true));
                }
                stream.skip_to_end();
                Some("comment")
            }
            quote @ ('"' | '\'') => {
                self.scan = Scan::Quoted(quote);
                Some(self.quoted(stream, quote))
            }
            '[' if matches!(stream.peek(), Some('[' | '=')) => {
                Some(self.open_bracket(stream, false))
            }
            '0'..='9' => {
                stream.eat_while(|ch| word(ch) || matches!(ch, '.' | '%'));
                Some("number")
            }
            ch if word(ch) => {
                stream.eat_while(|ch| word(ch) || matches!(ch, '\\' | '-' | '.'));
                Some("variable")
            }
            _ => None,
        }
    }

    fn open_bracket(&mut self, stream: &mut StringStream<'_>, comment: bool) -> &'static str {
        let level = remaining(stream)
            .bytes()
            .take_while(|&ch| ch == b'=')
            .count();
        stream.set_pos(stream.pos() + level).unwrap();
        stream.eat('[');
        self.scan = Scan::Bracketed { level, comment };
        self.bracketed(stream, level, comment)
    }

    fn bracketed(
        &mut self,
        stream: &mut StringStream<'_>,
        level: usize,
        comment: bool,
    ) -> &'static str {
        let mut closing = None;
        for ch in stream {
            closing = match (closing, ch) {
                (None, ']') => Some(0),
                (Some(count), '=') => Some(count + 1),
                (Some(count), ']') if count == level => {
                    self.scan = Scan::Base;
                    break;
                }
                _ => None,
            };
        }
        if comment { "comment" } else { "string" }
    }

    fn quoted(&mut self, stream: &mut StringStream<'_>, quote: char) -> &'static str {
        let mut escaped = false;
        for ch in stream {
            if ch == quote && !escaped {
                break;
            }
            escaped = !escaped && ch == '\\';
        }
        if !escaped {
            self.scan = Scan::Base;
        }
        "string"
    }
}
