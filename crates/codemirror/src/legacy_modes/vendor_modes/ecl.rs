use super::{
    Context, ScanMode, StringStream, close_comment, identifier, listed, number_tail,
    string_continues,
};

const KEYWORD: &str = "abs acos allnodes ascii asin asstring atan atan2 ave case choose choosen choosesets clustersize combine correlation cos cosh count covariance cron dataset dedup define denormalize distribute distributed distribution ebcdic enth error evaluate event eventextra eventname exists exp failcode failmessage fetch fromunicode getisvalid global graph group hash hash32 hash64 hashcrc hashmd5 having if index intformat isvalid iterate join keyunicode length library limit ln local log loop map matched matchlength matchposition matchtext matchunicode max merge mergejoin min nolocal nonempty normalize parse pipe power preload process project pull random range rank ranked realformat recordof regexfind regexreplace regroup rejected rollup round roundup row rowdiff sample set sin sinh sizeof soapcall sort sorted sqrt stepped stored sum table tan tanh thisnode topn tounicode transfer trim truncate typeof ungroup unicodeorder variance which workunit xmldecode xmlencode xmltext xmlunicode";
const VARIABLE: &str = "apply assert build buildindex evaluate fail keydiff keypatch loadxml nothor notify output parallel sequential soapcall wait";
const MODIFIER: &str = "__compressed__ all and any as atmost before beginc++ best between case const counter csv descend encrypt end endc++ endmacro except exclusive expire export extend false few first flat from full function group header heading hole ifblock import in interface joined keep keyed last left limit load local locale lookup macro many maxcount maxlength min skew module named nocase noroot noscan nosort not of only opt or outer overwrite packed partition penalty physicallength pipe quote record relationship repeat return right scan self separator service shared skew skip sql store terminator thor threshold token transform trim true type unicodeorder unsorted validate virtual whole wild within xml xpath";
const TYPE: &str = "ascii big_endian boolean data decimal ebcdic integer pattern qstring real record rule set of string token udecimal unicode unsigned varstring varunicode";
const BUILTIN: &str = "checkpoint deprecated failcode failmessage failure global independent onwarning persist priority recovery stored success wait when";
const BLOCK: &str = "catch class do else finally for if switch try while";
const ATOM: &str = "true false null";

#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    Top,
    Statement,
    Close(char),
}

#[derive(Clone, Copy)]
enum Punctuation {
    Char(char),
    NewStatement,
}

#[derive(Clone, Debug)]
pub(super) struct State {
    contexts: Vec<Context<Kind>>,
    initial_unit: usize,
    indented: usize,
    start_of_line: bool,
    scan: ScanMode,
}

impl State {
    pub(super) fn new(indent_unit: usize) -> Self {
        Self {
            contexts: vec![Context {
                align: Some(false),
                ..Context::new(Kind::Top, 0, 0)
            }],
            initial_unit: indent_unit,
            indented: 0,
            start_of_line: true,
            scan: ScanMode::Base,
        }
    }

    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        if stream.sol() {
            self.contexts.last_mut().unwrap().align.get_or_insert(false);
            self.indented = stream.indentation();
            self.start_of_line = true;
        }
        if stream.eat_space() {
            return None;
        }
        let (style, punctuation) = match self.scan {
            ScanMode::Base => self.base(stream),
            ScanMode::Comment => (Some(self.comment(stream)), None),
            ScanMode::String(quote) => (Some(self.string(stream, quote)), None),
        };
        if matches!(style, Some("comment" | "meta")) {
            return style;
        }
        let context = self.contexts.last_mut().unwrap();
        context.align.get_or_insert(true);
        let kind = context.kind;
        let push = match punctuation {
            Some(Punctuation::Char(':' | ';')) if kind == Kind::Statement => {
                self.pop();
                None
            }
            Some(Punctuation::Char('{')) => Some(Kind::Close('}')),
            Some(Punctuation::Char('[')) => Some(Kind::Close(']')),
            Some(Punctuation::Char('(')) => Some(Kind::Close(')')),
            Some(Punctuation::Char('}')) => {
                self.pop_statements();
                if self.contexts.last().unwrap().kind == Kind::Close('}') {
                    self.pop();
                }
                self.pop_statements();
                None
            }
            Some(Punctuation::Char(ch)) if kind == Kind::Close(ch) => {
                self.pop();
                None
            }
            _ if matches!(kind, Kind::Top | Kind::Close('}'))
                || (kind == Kind::Statement
                    && matches!(punctuation, Some(Punctuation::NewStatement))) =>
            {
                Some(Kind::Statement)
            }
            _ => None,
        };
        if let Some(kind) = push {
            self.contexts
                .push(Context::new(kind, self.indented, stream.column()));
        }
        self.start_of_line = false;
        style
    }

    fn base(
        &mut self,
        stream: &mut StringStream<'_>,
    ) -> (Option<&'static str>, Option<Punctuation>) {
        let Some(ch) = stream.next() else {
            return (None, None);
        };
        let style = match ch {
            '#' if self.start_of_line => {
                stream.skip_to_end();
                "meta"
            }
            '"' | '\'' => self.string(stream, ch),
            '[' | ']' | '{' | '}' | '(' | ')' | ',' | ';' | ':' | '.' => {
                return (None, Some(Punctuation::Char(ch)));
            }
            '0'..='9' => {
                stream.eat_while(number_tail);
                "number"
            }
            '/' if stream.eat('*').is_some() => self.comment(stream),
            '/' if stream.eat('/').is_some() => {
                stream.skip_to_end();
                "comment"
            }
            ch if operator(ch) => {
                stream.eat_while(operator);
                "operator"
            }
            _ => {
                stream.eat_while(identifier);
                let current = stream.current().to_lowercase();
                return word_style(&current);
            }
        };
        (Some(style), None)
    }

    fn string(&mut self, stream: &mut StringStream<'_>, quote: char) -> &'static str {
        self.scan = if string_continues(stream, quote) {
            ScanMode::String(quote)
        } else {
            ScanMode::Base
        };
        "string"
    }

    fn comment(&mut self, stream: &mut StringStream<'_>) -> &'static str {
        self.scan = if close_comment(stream, false) {
            ScanMode::Base
        } else {
            ScanMode::Comment
        };
        "comment"
    }

    fn pop(&mut self) {
        let context = self.contexts.pop().unwrap();
        if matches!(context.kind, Kind::Close(_)) {
            self.indented = context.indented;
        }
    }

    fn pop_statements(&mut self) {
        while self.contexts.last().unwrap().kind == Kind::Statement {
            self.pop();
        }
    }

    pub(super) fn indent(&self, text_after: &str, unit: usize) -> usize {
        if !matches!(self.scan, ScanMode::Base) {
            return 0;
        }
        let first = text_after.chars().next();
        let mut context = self.contexts.last().unwrap();
        if context.kind == Kind::Statement && first == Some('}') {
            context = &self.contexts[self.contexts.len() - 2];
        }
        match context.kind {
            Kind::Top => unit.saturating_sub(self.initial_unit),
            Kind::Statement => {
                context
                    .indented
                    .saturating_add(if first == Some('{') { 0 } else { unit })
            }
            Kind::Close(ch) => context.indent(first == Some(ch), unit),
        }
    }
}

fn operator(ch: char) -> bool {
    "+-*&%=<>!?|/".contains(ch)
}

fn word_style(current: &str) -> (Option<&'static str>, Option<Punctuation>) {
    for (list, style) in [
        (KEYWORD, "keyword"),
        (VARIABLE, "variable"),
        (MODIFIER, "modifier"),
        (TYPE, "type"),
        (BUILTIN, "builtin"),
    ] {
        if listed(list, current) {
            let punctuation = listed(BLOCK, current).then_some(Punctuation::NewStatement);
            return (Some(style), punctuation);
        }
    }
    let base = current.trim_end_matches(|ch: char| ch.is_ascii_digit() || ch == '_');
    if base.len() > 1 && listed(TYPE, base) {
        return (Some("type"), None);
    }
    (listed(ATOM, current).then_some("atom"), None)
}
