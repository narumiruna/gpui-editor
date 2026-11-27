use super::{js_state, prefix, remaining, web_modes};
use crate::{
    Text, language::javascript::JavascriptDocument, legacy_modes::js_space,
    stream_parser::StringStream,
};

#[derive(Debug)]
pub(super) struct State {
    scan: Scan,
    mixin_call_after: bool,
}

#[derive(Clone, Debug)]
struct Scan {
    script: web_modes::State,
    line_script: Option<bool>,
    arguments_depth: Option<usize>,
    interpolation_depth: Option<usize>,
    rest_of_line: Option<&'static str>,
    each: bool,
    attributes: Option<Attributes>,
}

#[derive(Clone, Debug)]
struct Attributes {
    closers: Vec<char>,
    in_name: bool,
    value: String,
}

impl Clone for State {
    fn clone(&self) -> Self {
        // Upstream copyState omits this transient flag.
        Self {
            scan: self.scan.clone(),
            mixin_call_after: false,
        }
    }
}

impl State {
    pub(super) fn new(indent_unit: usize) -> Self {
        Self {
            scan: Scan {
                script: js_state(indent_unit),
                line_script: None,
                arguments_depth: None,
                interpolation_depth: None,
                rest_of_line: None,
                each: false,
                attributes: None,
            },
            mixin_call_after: false,
        }
    }

    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        let scan = &mut self.scan;
        if let Some(style) = scan.rest_of_line.take().filter(|_| !stream.sol()) {
            stream.skip_to_end();
            return Some(style);
        }
        if let Some(depth) = &mut scan.interpolation_depth {
            match stream.peek() {
                Some('}') if *depth == 0 => {
                    stream.next();
                    scan.interpolation_depth = None;
                    return Some("punctuation");
                }
                Some('}') => *depth -= 1,
                Some('{') => *depth += 1,
                _ => {}
            }
            return scan.script.token(stream);
        }
        if scan.each {
            if prefix(stream, re!(r"^ in\b")) {
                scan.line_script = Some(false);
                scan.each = false;
                return Some("keyword");
            }
            if stream.sol() || stream.eol() {
                scan.each = false;
            } else {
                while stream.next().is_some() {
                    if re!(r"^ in\b").find(remaining(stream)).is_some() {
                        break;
                    }
                }
                return Some("variable");
            }
        }
        if let Some(attrs) = &mut scan.attributes {
            return match attrs.token(stream, &mut scan.script) {
                AttributeToken::Style(style) => style,
                AttributeToken::Close => {
                    scan.attributes = None;
                    Some("punctuation")
                }
            };
        }
        if stream.sol() || scan.line_script == Some(true) && stream.peek() == Some(':') {
            scan.line_script = None;
        }
        if scan.line_script.is_some() {
            let style = scan.script.token(stream);
            if stream.eol() {
                scan.line_script = None;
            }
            return style;
        }
        if let Some(mut depth) = scan.arguments_depth {
            match stream.peek() {
                Some('(') => depth += 1,
                Some(')') if depth != 0 => depth -= 1,
                _ => {}
            }
            scan.arguments_depth = (depth != 0).then_some(depth);
            if scan.arguments_depth.is_some() {
                return scan.script.token(stream);
            }
        }
        if std::mem::take(&mut self.mixin_call_after) {
            scan.enable_arguments(stream);
            return None;
        }

        if prefix(stream, re!(r"^yield\b")) {
            return Some("keyword");
        }
        if prefix(stream, re!(r"^doctype *[^\n]*")) {
            return Some("meta");
        }
        if stream.match_str("#{", true, false) {
            scan.interpolation_depth = Some(0);
            return Some("punctuation");
        }
        if prefix(stream, re!(r"^case\b")) {
            scan.line_script = Some(false);
            return Some("keyword");
        }
        if prefix(stream, re!(r"^when\b")) {
            scan.line_script = Some(true);
            return Some("keyword");
        }
        if prefix(stream, re!(r"^default\b")) {
            return Some("keyword");
        }
        // `include` takes priority even when followed by a filter colon.
        if prefix(stream, re!(r"^(?:extends?|include)\b")) {
            scan.rest_of_line = Some("string");
            return Some("keyword");
        }
        if prefix(
            stream,
            re!(r"^(?:append|prepend)\b|^block\b *(?:(?:prepend|append)\b)?"),
        ) {
            scan.rest_of_line = Some("variable");
            return Some("keyword");
        }
        if prefix(stream, re!(r"^mixin\b")) {
            scan.line_script = Some(false);
            return Some("keyword");
        }
        if prefix(stream, re!(r"^\+[-\w]+")) {
            scan.enable_arguments(stream);
            return Some("variable");
        }
        if stream.match_str("+#{", true, false) {
            self.mixin_call_after = true;
            scan.interpolation_depth = Some(0);
            return Some("punctuation");
        }
        if prefix(stream, re!(r"^(?:if|unless|else if|else)\b")) {
            scan.line_script = Some(false);
            return Some("keyword");
        }
        if prefix(stream, re!(r"^(?:- *)?(?:each|for)\b")) {
            scan.each = true;
            return Some("keyword");
        }
        if prefix(stream, re!(r"^while\b")) {
            scan.line_script = Some(false);
            return Some("keyword");
        }
        if prefix(stream, re!(r"^\w(?:[-:\w]*\w)?/?")) {
            return Some("tag");
        }
        if prefix(stream, re!(r"^:[\w\-]+")) {
            return Some("atom");
        }
        if prefix(stream, re!(r"^(?:!?=|-)")) {
            scan.line_script = Some(false);
            return Some("punctuation");
        }
        if prefix(stream, re!(r"^#[\w-]+")) {
            return Some("builtin");
        }
        if prefix(stream, re!(r"^\.[\w-]+")) {
            return Some("className");
        }
        if stream.eat('(').is_some() {
            scan.attributes = Some(Attributes {
                closers: Vec::new(),
                in_name: true,
                value: String::new(),
            });
            return Some("punctuation");
        }
        if prefix(stream, re!(r"^&attributes\b")) {
            scan.arguments_depth = Some(0);
            return Some("keyword");
        }
        if stream.sol() && stream.eat_space() {
            return Some("indent");
        }
        if prefix(stream, re!(r"^(?:\| ?| )[^\n]+")) {
            return Some("string");
        }
        if stream.peek() == Some('<') {
            stream.skip_to_end();
            return Some("string");
        }
        if prefix(stream, re!(r"^ *//-?[^\n]*")) {
            return Some("comment");
        }
        if prefix(stream, re!(r"^: *")) {
            return Some("colon");
        }
        if stream.eat('.').is_some() {
            return Some("dot");
        }
        stream.next();
        None
    }
}

impl Scan {
    fn enable_arguments(&mut self, stream: &StringStream<'_>) {
        if re!(r"^\( *[-\w]+ *=").find(remaining(stream)).is_none() {
            self.arguments_depth = Some(0);
        }
    }
}

enum AttributeToken {
    Style(Option<&'static str>),
    Close,
}

impl Attributes {
    fn token(
        &mut self,
        stream: &mut StringStream<'_>,
        script: &mut web_modes::State,
    ) -> AttributeToken {
        // Reclassify a prospective JavaScript token as the next attribute name.
        // The JavaScript state is not rewound, just as in the upstream scanner.
        loop {
            let closer = match stream.peek() {
                Some('{') => Some('}'),
                Some('(') => Some(')'),
                Some('[') => Some(']'),
                _ => None,
            };
            if let Some(closer) = closer {
                self.closers.push(closer);
            }
            if self
                .closers
                .last()
                .is_some_and(|&ch| stream.peek() == Some(ch))
            {
                self.closers.pop();
            } else if stream.eat(')').is_some() {
                return AttributeToken::Close;
            }
            if self.in_name && stream.eat_while(|ch| !matches!(ch, '=' | ',' | ')' | '!')) {
                if matches!(stream.peek(), Some('=' | '!')) {
                    self.in_name = false;
                    *script = js_state(2);
                }
                return AttributeToken::Style(Some("attribute"));
            }

            let style = script.token(stream);
            if !self.in_name
                && self.closers.is_empty()
                && matches!(style, Some("string" | "variable" | "keyword"))
                && valid_binding(&self.value)
            {
                self.in_name = true;
                self.value.clear();
                stream.set_pos(stream.start()).unwrap();
            } else {
                self.value.push_str(stream.current());
                return AttributeToken::Style(style);
            }
        }
    }
}

fn valid_binding(value: &str) -> bool {
    let value = value
        .trim_end_matches(js_space)
        .strip_suffix(',')
        .unwrap_or(value);
    let value = value.strip_prefix('!').unwrap_or(value);
    // Upstream uses a Function constructor to check whether `var x …` parses.
    // Use the native JavaScript parser and do not evaluate the attribute.
    !JavascriptDocument::parse(Text::new(format!("var x {value}"))).is_recovered()
}
