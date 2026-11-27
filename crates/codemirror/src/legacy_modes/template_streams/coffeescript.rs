use super::{prefix, remaining};
use crate::stream_parser::StringStream;

const INDENT_KEYWORDS: &str = "for|while|loop|if|unless|else|switch|try|catch|finally|class";
const COMMON_KEYWORDS: &str =
    "break|by|continue|debugger|delete|do|in|of|new|return|then|this|@|throw|when|until|extends";
const IDENTIFIER: &str = r"[_A-Za-z$][_A-Za-z$0-9]*";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Scan {
    #[default]
    Base,
    Comment,
    Literal {
        end: &'static str,
        regex: bool,
    },
}

#[derive(Clone, Copy, Debug)]
struct Scope {
    offset: usize,
    closer: Option<char>,
    align: Option<bool>,
    align_offset: usize,
}

#[derive(Clone, Debug)]
pub(super) struct State {
    scan: Scan,
    scopes: Vec<Scope>,
    property: bool,
    dedent_at_eol: bool,
}

impl Default for State {
    fn default() -> Self {
        Self {
            scan: Scan::Base,
            scopes: vec![Scope {
                offset: 0,
                closer: None,
                align: Some(false),
                align_offset: 0,
            }],
            property: false,
            dedent_at_eol: false,
        }
    }
}

impl State {
    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        let fill_align = self.scopes.len() - 1;
        let pending = self.scopes[fill_align].align.is_none();
        if pending && stream.sol() {
            self.scopes[fill_align].align = Some(false);
        }
        let mut style = match self.scan {
            Scan::Base => self.base(stream),
            Scan::Comment => Some(self.comment(stream)),
            Scan::Literal { end, regex } => Some(self.literal(stream, end, regex)),
        };
        // The base scanner can remove the old scope at the start of a line.
        let pending = pending && fill_align < self.scopes.len();
        let token = stream.current();
        if token == "return" {
            self.dedent_at_eol = true;
        }
        if (matches!(token, "->" | "=>") && stream.eol()) || style == Some("indent") {
            self.push_scope(stream, None);
        }
        let close = match token {
            "[" => Some(']'),
            "(" => Some(')'),
            "{" => Some('}'),
            _ => None,
        };
        if close.is_some() {
            self.push_scope(stream, close);
        }
        if re!(&format!(r"^(?:{INDENT_KEYWORDS})\b"))
            .find(token)
            .is_some()
        {
            self.push_scope(stream, None);
        }
        if token == "then" {
            self.dedent(stream);
        }
        if style == Some("dedent") && self.dedent(stream) {
            style = Some("error");
        }
        if matches!(token, "]" | ")" | "}") {
            let scope = &self.scopes[self.bracket_scope()];
            let is_match = scope.closer == token.chars().next();
            self.scopes
                .truncate(self.bracket_scope() + usize::from(!is_match));
        }
        if self.dedent_at_eol && stream.eol() {
            if self.scopes.len() > 1 && self.top().closer.is_none() {
                self.scopes.pop();
            }
            self.dedent_at_eol = false;
        }
        if matches!(style, Some("indent" | "dedent")) {
            return None;
        }
        if style.is_some_and(|style| style != "comment") {
            if pending && let Some(scope) = self.scopes.get_mut(fill_align) {
                scope.align = Some(true);
            }
            self.property = style == Some("punctuation") && token == ".";
        }
        style
    }

    pub(super) fn indent(&self, text: &str) -> usize {
        if self.scan != Scan::Base {
            return 0;
        }
        let first = text.chars().next().filter(|ch| "])}".contains(*ch));
        let index = if first.is_some() {
            self.bracket_scope()
        } else {
            self.scopes.len() - 1
        };
        let scope = self.scopes[index];
        let closes = first.is_some() && first == scope.closer;
        if scope.align == Some(true) {
            scope.align_offset.saturating_sub(usize::from(closes))
        } else {
            self.scopes[index - usize::from(closes)].offset
        }
    }

    fn top(&self) -> Scope {
        *self.scopes.last().unwrap()
    }

    fn bracket_scope(&self) -> usize {
        self.scopes
            .iter()
            .rposition(|scope| scope.closer.is_some())
            .unwrap_or(0)
    }

    fn push_scope(&mut self, stream: &StringStream<'_>, closer: Option<char>) {
        let base = self
            .scopes
            .iter()
            .rev()
            .find(|scope| scope.closer.is_none() || scope.closer == Some('}'))
            .unwrap();
        let offset = base.offset.saturating_add(stream.indent_unit());
        if closer.is_none() && self.top().align == Some(true) {
            self.scopes.last_mut().unwrap().align = Some(false);
        }
        self.scopes.push(Scope {
            offset,
            closer,
            align: if closer.is_none() { Some(false) } else { None },
            align_offset: stream.column() + stream.current().chars().count(),
        });
    }

    // Return true for a dedent that does not match a known offset.
    fn dedent(&mut self, stream: &StringStream<'_>) -> bool {
        if self.scopes.len() == 1 {
            return false;
        }
        if self.top().closer.is_some() {
            self.scopes.pop();
        } else if let Some(index) = self
            .scopes
            .iter()
            .rposition(|scope| scope.offset == stream.indentation())
        {
            self.scopes.truncate(index + 1);
        } else {
            return true;
        }
        false
    }

    fn base(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        if stream.sol() {
            self.scopes.last_mut().unwrap().align.get_or_insert(false);
            let scope = self.top();
            if stream.eat_space() {
                let indent = stream.indentation();
                return if indent > scope.offset && scope.closer.is_none() {
                    Some("indent")
                } else if indent < scope.offset {
                    Some("dedent")
                } else {
                    None
                };
            }
            if scope.offset > 0 {
                self.dedent(stream);
            }
        } else if stream.eat_space() {
            return None;
        }

        if stream.match_str("####", true, false) {
            stream.skip_to_end();
            return Some("comment");
        }
        if stream.match_str("###", true, false) {
            self.scan = Scan::Comment;
            return Some(self.comment(stream));
        }
        if stream.peek() == Some('#') {
            stream.skip_to_end();
            return Some("comment");
        }
        if number(stream) {
            return Some("number");
        }
        for end in ["'''", "\"\"\"", "'", "\""] {
            if stream.match_str(end, true, false) {
                self.scan = Scan::Literal { end, regex: false };
                return Some(self.literal(stream, end, false));
            }
        }
        if stream.match_str("///", true, false) {
            self.scan = Scan::Literal {
                end: "///",
                regex: true,
            };
            return Some(self.literal(stream, "///", true));
        }
        if stream.peek() == Some('/') && re!(r"^/.*\/").find(remaining(stream)).is_some() {
            stream.next();
            self.scan = Scan::Literal {
                end: "/",
                regex: true,
            };
            return Some(self.literal(stream, "/", true));
        }

        let style = if prefix(
            stream,
            re!(
                r"^(?:->|=>|\+[+=]?|-[\-=]?|\*[\*=]?|/[\/=]?|[=!]=|<[><]?=?|>>?=?|%=?|&=?|\|=?|\^=?|\~|!|\?|(or|and|\|\||&&|\?)=)"
            ),
        ) || prefix(
            stream,
            re!(r"^(?:and|or|not|is|isnt|in|instanceof|typeof)\b"),
        ) {
            "operator"
        } else if prefix(stream, re!(r"^(?:[()\[\]{},:`=;]|\.\.?\.?)")) {
            "punctuation"
        } else if prefix(
            stream,
            re!(r"^(?:Infinity|NaN|undefined|null|true|false|on|off|yes|no)\b"),
        ) {
            "atom"
        } else if prefix(stream, re!(&format!("^@{IDENTIFIER}")))
            || self.property && prefix(stream, re!(&format!("^{IDENTIFIER}")))
        {
            "property"
        } else if prefix(
            stream,
            re!(&format!(r"^(?:{INDENT_KEYWORDS}|{COMMON_KEYWORDS})\b")),
        ) {
            "keyword"
        } else if prefix(stream, re!(&format!("^{IDENTIFIER}"))) {
            "variable"
        } else {
            stream.next();
            "error"
        };
        Some(style)
    }

    fn comment(&mut self, stream: &mut StringStream<'_>) -> &'static str {
        while !stream.eol() {
            stream.eat_while(|ch| ch != '#');
            if stream.match_str("###", true, false) {
                self.scan = Scan::Base;
                break;
            }
            stream.eat_while(|ch| ch == '#');
        }
        "comment"
    }

    fn literal(&mut self, stream: &mut StringStream<'_>, end: &str, regex: bool) -> &'static str {
        let style = if regex { "string.special" } else { "string" };
        while !stream.eol() {
            stream.eat_while(|ch| !matches!(ch, '\'' | '"' | '/' | '\\'));
            if stream.eat('\\').is_some() {
                stream.next();
                if regex && stream.eol() {
                    return style;
                }
            } else if stream.match_str(end, true, false) {
                self.scan = Scan::Base;
                return style;
            } else {
                stream.eat_if(|ch| matches!(ch, '\'' | '"' | '/'));
            }
        }
        if regex {
            self.scan = Scan::Base;
        }
        style
    }
}

fn number(stream: &mut StringStream<'_>) -> bool {
    if re!(r"^-?[0-9.]").find(remaining(stream)).is_none() {
        return false;
    }
    // Upstream tries all three float rules in order at the current cursor.
    let mut float = prefix(stream, re!(r"^-?\d*\.\d+([eE][+\-]?\d+)?"));
    float |= prefix(stream, re!(r"^-?\d+\.\d*"));
    float |= prefix(stream, re!(r"^-?\.\d+"));
    if float {
        if stream.peek() == Some('.') {
            stream.back_up(1).unwrap();
        }
        return true;
    }
    let mut integer = prefix(stream, re!(r"^-?0[xX][0-9a-fA-F]+"));
    integer |= prefix(stream, re!(r"^-?[1-9]\d*(e[+\-]?\d+)?"));
    integer |= prefix(stream, re!(r"^-?0(?![\dxX])"));
    integer
}
