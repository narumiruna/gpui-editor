use std::{collections::HashSet, sync::LazyLock};

use regex::Regex;

use super::{Contexts, Kind, bracket_close, identifier, tlv, word};
use crate::stream_parser::StringStream;

const KEYWORDS: &str = concat!(
    "accept_on alias always always_comb always_ff always_latch and assert assign assume automatic before begin bind ",
    "bins binsof bit break buf bufif0 bufif1 byte case casex casez cell chandle checker class clocking cmos config ",
    "const constraint context continue cover covergroup coverpoint cross deassign default defparam design disable ",
    "dist do edge else end endcase endchecker endclass endclocking endconfig endfunction endgenerate endgroup ",
    "endinterface endmodule endpackage endprimitive endprogram endproperty endspecify endsequence endtable endtask ",
    "enum event eventually expect export extends extern final first_match for force foreach forever fork forkjoin ",
    "function generate genvar global highz0 highz1 if iff ifnone ignore_bins illegal_bins implements implies import ",
    "incdir include initial inout input inside instance int integer interconnect interface intersect join join_any ",
    "join_none large let liblist library local localparam logic longint macromodule matches medium modport module ",
    "nand negedge nettype new nexttime nmos nor noshowcancelled not notif0 notif1 null or output package packed ",
    "parameter pmos posedge primitive priority program property protected pull0 pull1 pulldown pullup ",
    "pulsestyle_ondetect pulsestyle_onevent pure rand randc randcase randsequence rcmos real realtime ref reg ",
    "reject_on release repeat restrict return rnmos rpmos rtran rtranif0 rtranif1 s_always s_eventually s_nexttime ",
    "s_until s_until_with scalared sequence shortint shortreal showcancelled signed small soft solve specify ",
    "specparam static string strong strong0 strong1 struct super supply0 supply1 sync_accept_on sync_reject_on ",
    "table tagged task this throughout time timeprecision timeunit tran tranif0 tranif1 tri tri0 tri1 triand trior ",
    "trireg type typedef union unique unique0 unsigned until until_with untyped use uwire var vectored virtual void ",
    "wait wait_order wand weak weak0 weak1 while wildcard wire with within wor xnor xor",
);

const BLOCKS: &[(&str, &str)] = &[
    ("begin", "end"),
    ("case", "endcase"),
    ("casex", "endcase"),
    ("casez", "endcase"),
    ("checker", "endchecker"),
    ("class", "endclass"),
    ("clocking", "endclocking"),
    ("config", "endconfig"),
    ("covergroup", "endgroup"),
    ("do", "while"),
    ("fork", "join;join_any;join_none"),
    ("function", "endfunction"),
    ("generate", "endgenerate"),
    ("interface", "endinterface"),
    ("module", "endmodule"),
    ("package", "endpackage"),
    ("primitive", "endprimitive"),
    ("program", "endprogram"),
    ("property", "endproperty"),
    ("sequence", "endsequence"),
    ("specify", "endspecify"),
    ("table", "endtable"),
    ("task", "endtask"),
];

const STATEMENTS: &str = concat!(
    "always always_comb always_ff always_latch assert assign assume else export for ",
    "foreach forever if import initial repeat while",
);

// The upstream keyword tables also match inherited JavaScript object keys.
const OBJECT_KEYS: &str = concat!(
    "__defineGetter__ __defineSetter__ __lookupGetter__ __lookupSetter__ __proto__ ",
    "constructor hasOwnProperty isPrototypeOf propertyIsEnumerable toLocaleString toString valueOf",
);

static KEYWORD_SET: LazyLock<HashSet<&str>> =
    LazyLock::new(|| KEYWORDS.split_ascii_whitespace().collect());

// Preserve the upstream alternative order and ECMAScript whitespace set.
const SPACE: &str = r"[\x09-\x0D\x20\u{A0}\u{1680}\u{2000}-\u{200A}\u{2028}\u{2029}\u{202F}\u{205F}\u{3000}\u{FEFF}]";
static NUMBER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        concat!(
            r"^(?:[0-9][0-9_]*(?:\.[0-9][0-9_]*)?[Ee]-?[0-9_]+|[0-9][0-9_]*\.[0-9][0-9_]*|",
            r"[0-9]*{0}*'[sS]?[dD]{0}*[0-9][0-9_]*|",
            r"[0-9]*{0}*'[sS]?[bB]{0}*[xzXZ01][xzXZ01_]*|",
            r"[0-9]*{0}*'[sS]?[oO]{0}*[xzXZ0-7][xzXZ0-7_]*|",
            r"[0-9]*{0}*'[sS]?[hH]{0}*[0-9a-fA-FxzXZ?][0-9a-fA-FxzXZ?_]*|[0-9][0-9_]*)",
        ),
        SPACE,
    ))
    .unwrap()
});

#[derive(Clone, Copy, Debug)]
enum Scan {
    Base,
    String,
    Comment,
}

#[derive(Clone, Debug)]
pub(super) struct State {
    scan: Scan,
    contexts: Contexts,
}

type Token<'a> = (Option<&'static str>, Option<Event<'a>>);

#[derive(Clone, Copy)]
enum Event<'a> {
    Punct(char),
    Keyword(&'a str),
}

impl State {
    pub(super) fn new(unit: usize) -> Self {
        Self {
            scan: Scan::Base,
            contexts: Contexts::new(unit),
        }
    }

    pub(super) fn token(
        &mut self,
        stream: &mut StringStream<'_>,
        tlv: Option<&mut tlv::State>,
    ) -> Option<&'static str> {
        self.contexts.start_line(stream);
        let mut consumed_astral = false;
        if let Some(tlv) = tlv {
            let start = stream.pos();
            if let Some(style) = tlv.token(stream, &mut self.contexts.indented) {
                return Some(style);
            }
            // On fallthrough, the upstream base scanner still sees the low
            // surrogate. Do not consume the scalar after it as part of this token.
            consumed_astral =
                stream.line()[start..stream.pos()].starts_with(|ch: char| ch.len_utf16() == 2);
        }
        if !consumed_astral && stream.eat_space() {
            return None;
        }
        let (style, event) = match self.scan {
            Scan::Base if consumed_astral => (None, None),
            Scan::Base => self.base(stream),
            Scan::String => (Some(self.string(stream)), None),
            Scan::Comment => (Some(self.comment(stream)), None),
        };
        if matches!(style, Some("comment" | "meta" | "variable")) {
            return style;
        }
        self.contexts.set_alignment(true);
        let kind = self.contexts.current().kind;
        match event {
            Some(Event::Punct(ch)) if kind == Kind::Bracket(ch) => self.contexts.pop(),
            Some(Event::Punct(';')) if kind == Kind::Statement => self.contexts.pop_statements(),
            Some(Event::Punct(':')) if kind == Kind::Block("endcase") => {
                self.contexts.push(Kind::Statement, stream);
            }
            Some(Event::Punct(ch)) => {
                if let Some(close) = bracket_close(ch) {
                    self.contexts.push(Kind::Bracket(close), stream);
                }
            }
            Some(Event::Keyword(text)) if is_closing(kind, text) => {
                self.contexts.pop();
                self.contexts.pop_statements();
            }
            Some(Event::Keyword(text)) => {
                if STATEMENTS.split_ascii_whitespace().any(|word| word == text)
                    || OBJECT_KEYS
                        .split_ascii_whitespace()
                        .any(|word| word == text)
                {
                    self.contexts.push(Kind::Statement, stream);
                } else if !matches!(
                    (text, kind),
                    ("function", Kind::Statement | Kind::Block("endgroup"))
                        | ("task", Kind::Statement)
                ) && let Some((_, close)) = BLOCKS.iter().find(|(open, _)| *open == text)
                {
                    self.contexts.push(Kind::Block(close), stream);
                }
            }
            None => {}
        }
        style
    }

    fn base<'a>(&mut self, stream: &mut StringStream<'a>) -> Token<'a> {
        let Some(ch) = stream.peek() else {
            return (None, None);
        };
        let style = match ch {
            ',' | ';' | ':' | '.' => {
                stream.next();
                return (None, Some(Event::Punct(ch)));
            }
            '(' | ')' | '[' | ']' | '{' | '}' => {
                stream.next();
                return (Some("bracket"), Some(Event::Punct(ch)));
            }
            '`' | '$' => {
                stream.next();
                stream
                    .eat_while(identifier)
                    .then_some(if ch == '`' { "def" } else { "meta" })
            }
            '#' => {
                stream.next();
                stream.eat_while(|ch| ch.is_ascii_digit() || matches!(ch, '_' | '.'));
                Some("def")
            }
            '"' => {
                stream.next();
                Some(self.string(stream))
            }
            '/' if stream.match_str("/*", true, false) => Some(self.comment(stream)),
            '/' if stream.match_str("//", true, false) => {
                stream.skip_to_end();
                Some("comment")
            }
            _ if matches!(ch, '0'..='9' | '\'') && stream.match_regex(&NUMBER, true).is_some() => {
                Some("number")
            }
            _ if stream.eat_while(|ch| "+-*/!~&|^%=?:".contains(ch)) => Some("meta"),
            _ if stream.eat_while(identifier) => {
                let text = stream.current();
                if KEYWORD_SET.contains(text)
                    || OBJECT_KEYS
                        .split_ascii_whitespace()
                        .any(|word| word == text)
                {
                    return (Some("keyword"), Some(Event::Keyword(text)));
                }
                Some("variable")
            }
            _ => {
                stream.next();
                None
            }
        };
        (style, None)
    }

    fn string(&mut self, stream: &mut StringStream<'_>) -> &'static str {
        let mut escaped = false;
        for ch in stream {
            if ch == '"' && !escaped {
                self.scan = Scan::Base;
                return "string";
            }
            escaped = !escaped && ch == '\\';
        }
        self.scan = if escaped { Scan::String } else { Scan::Base };
        "string"
    }

    fn comment(&mut self, stream: &mut StringStream<'_>) -> &'static str {
        if stream.skip_to("*/") {
            stream.match_str("*/", true, false);
            self.scan = Scan::Base;
        } else {
            stream.skip_to_end();
            self.scan = Scan::Comment;
        }
        "comment"
    }

    pub(super) fn indent(
        &self,
        text_after: &str,
        unit: usize,
        tlv: Option<usize>,
    ) -> Option<usize> {
        if !matches!(self.scan, Scan::Base) {
            return None;
        }
        if tlv.is_some() {
            return tlv;
        }
        let mut ctx = self.contexts.current();
        if ctx.kind == Kind::Statement && text_after.starts_with('}') {
            ctx = self.contexts.outer();
        }
        let possible_closing = if text_after.starts_with([')', '}', ']']) {
            &text_after[..1]
        } else {
            &text_after[..text_after.find(|ch| !word(ch)).unwrap_or(text_after.len())]
        };
        let closing = is_closing(ctx.kind, possible_closing);
        Some(match ctx.kind {
            Kind::Statement => {
                ctx.indented
                    .saturating_add(if text_after.starts_with('{') { 0 } else { unit })
            }
            Kind::Bracket(_) if ctx.align == Some(true) => {
                ctx.column.saturating_add(usize::from(!closing))
            }
            _ => self
                .contexts
                .indent_from(ctx, if closing { 0 } else { unit }),
        })
    }
}

fn is_closing(kind: Kind, text: &str) -> bool {
    match kind {
        Kind::Top => text == "top",
        Kind::Statement => text == "statement",
        Kind::Bracket(ch) => text.starts_with(ch),
        Kind::Block(close) => close.split(';').any(|close| text == close),
    }
}

pub(super) fn indent_on_input(text: &str) -> bool {
    text.contains(['{', '}', '(', ')', '[', ']'])
        || BLOCKS
            .iter()
            .flat_map(|(_, close)| close.split(';'))
            .any(|close| text.ends_with(close))
}
