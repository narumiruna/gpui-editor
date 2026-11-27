use crate::stream_parser::StringStream;

use super::{
    Phase, State, end_of_line, first_word, is_block, is_property, is_tag, match_pattern, rest,
    scanner::{Kind, Kind::Mark, Style},
    start_of_line, words,
};

impl State {
    pub(super) fn parse(
        &mut self,
        mut phase: Phase,
        kind: Kind,
        stream: &mut StringStream<'_>,
        style: &mut Style,
    ) -> Phase {
        loop {
            match phase {
                Phase::Block => return self.block(kind, stream, style),
                Phase::Parens => return self.parens(kind, stream, style),
                Phase::VendorPrefixes => {
                    if kind == Kind::Word {
                        *style = Some("property");
                        return self.push(Phase::Block, stream, 0);
                    }
                    return self.pop();
                }
                Phase::Pseudo => {
                    if !is_property(first_word(stream.line())) {
                        match_pattern(stream, pattern!(r"^[a-z-]+"));
                        *style = Some("variableName.special");
                        return if end_of_line(stream) {
                            self.push(Phase::Block, stream, stream.indent_unit())
                        } else {
                            self.pop()
                        };
                    }
                }
                Phase::AtBlock => return self.at_block(kind, stream, style),
                Phase::AtBlockParens => {
                    return match kind {
                        Mark('{' | '}') => self.context().phase,
                        Mark(')') => {
                            let phase = if end_of_line(stream) {
                                Phase::Block
                            } else {
                                Phase::AtBlock
                            };
                            self.push(phase, stream, stream.indent_unit())
                        }
                        Kind::Word => {
                            let word = stream.current().to_ascii_lowercase();
                            let as_value = word_as_value(&word);
                            *style = Some(if word.starts_with("min") || word.starts_with("max") {
                                "property"
                            } else if as_value == "tag" {
                                if is_tag_variable(&word) {
                                    "variable"
                                } else {
                                    "atom"
                                }
                            } else {
                                as_value
                            });
                            self.context().phase
                        }
                        _ => self.at_block(kind, stream, style),
                    };
                }
                Phase::Keyframes => {
                    if !(stream.indentation() == 0
                        && (kind == Mark('}') && start_of_line(stream)
                            || matches!(kind, Mark(']') | Kind::Hash | Kind::Qualifier)
                            || is_tag(stream.current())))
                    {
                        return self.keyframes(kind, stream, style);
                    }
                }
                Phase::Interpolation => return self.interpolation(kind, stream, style),
                Phase::Extend => {
                    return match kind {
                        Mark('[' | '=') => Phase::Extend,
                        Kind::Word => {
                            *style = Some(word_as_value(stream.current()));
                            Phase::Extend
                        }
                        _ => self.pop(),
                    };
                }
                Phase::VariableName => {
                    if matches!(kind, Kind::String | Mark('[' | ']'))
                        || stream.current().starts_with(['.', '$'])
                    {
                        if pattern!(r"^\.[\w-]+", "i").find(stream.current()).is_some() {
                            *style = Some("variable");
                        }
                        return Phase::VariableName;
                    }
                }
            }
            // These phases give the same token back to their parent.
            phase = self.pop();
        }
    }

    fn block(&mut self, kind: Kind, stream: &StringStream<'_>, style: &mut Style) -> Phase {
        let end = end_of_line(stream);
        let unit = stream.indent_unit();
        let line = stream.line();
        let word = first_word(line);
        if kind == Kind::Comment && start_of_line(stream)
            || kind == Mark(',') && end
            || kind == Kind::Mixin
        {
            return self.push(Phase::Block, stream, 0);
        }
        if is_interpolation(kind, stream) {
            return self.push(Phase::Interpolation, stream, unit);
        }
        if kind == Mark(']') && end && !pattern!(r"^\s*[.#:\[*&]").is_match(line) && !is_tag(word) {
            return self.push(Phase::Block, stream, 0);
        }
        if is_block_token(kind, stream) {
            return self.push(Phase::Block, stream, unit);
        }
        if kind == Mark('}') && end {
            return self.push(Phase::Block, stream, 0);
        }
        if kind == Kind::VariableName {
            let indent = if is_placeholder(line) || is_block(word) {
                unit
            } else {
                0
            };
            return self.push(Phase::VariableName, stream, indent);
        }
        if kind == Mark('=') {
            let indent = if !end && !is_block(word) { 0 } else { unit };
            return self.push(Phase::Block, stream, indent);
        }
        if kind == Mark('*') && (end || pattern!(r"^\s*[,\.\#\[:{]").is_match(rest(stream))) {
            *style = Some("tag");
            return self.push(Phase::Block, stream, unit);
        }
        if is_pseudo(kind, stream) {
            return self.push(Phase::Pseudo, stream, unit);
        }
        if kind == Kind::Def {
            let def = stream.current();
            if is_at_block(def) {
                return self.push(
                    if end { Phase::Block } else { Phase::AtBlock },
                    stream,
                    unit,
                );
            }
            if pattern!(r"@(?:-(?:moz|ms|o|webkit)-)?keyframes$").is_match(def) {
                return self.push(Phase::Keyframes, stream, unit);
            }
            if def.contains("@extend") {
                return self.push(Phase::Extend, stream, 0);
            }
            if stream.indentation() > 0 && is_property(&def[1..]) {
                *style = Some("variable");
                return Phase::Block;
            }
            let indent = if pattern!(r"@(?:import|require|charset)").is_match(def) {
                0
            } else {
                unit
            };
            return self.push(Phase::Block, stream, indent);
        }
        if kind == Kind::Reference && end {
            return self.push(Phase::Block, stream, unit);
        }
        if kind == Mark('(') {
            return self.push(Phase::Parens, stream, unit);
        }
        if kind == Kind::VendorPrefix {
            return self.push(Phase::VendorPrefixes, stream, unit);
        }
        if kind == Kind::Word {
            let current = stream.current();
            let value = word_as_value(current);
            *style = Some(value);
            match value {
                "property" => {
                    if start_of_line(stream) {
                        return self.push(Phase::Block, stream, 0);
                    }
                    *style = Some("atom");
                    return Phase::Block;
                }
                "tag" => {
                    if pattern!(r"embed|menu|pre|progress|sub|table").is_match(current)
                        && is_property(word)
                        || is_attribute(line, current)
                    {
                        *style = Some("atom");
                        return Phase::Block;
                    }
                    if is_tag_variable(current)
                        && (start_of_line(stream) && line.contains('=')
                            || !start_of_line(stream)
                                && !pattern!(r"^(?:\s*\.|#|&|\[|/|>|\*)").is_match(line)
                                && !is_tag(word))
                    {
                        *style = Some("variable");
                        if is_block(word) {
                            return Phase::Block;
                        }
                        return self.push(Phase::Block, stream, 0);
                    }
                    if end {
                        return self.push(Phase::Block, stream, unit);
                    }
                }
                "block-keyword" => {
                    *style = Some("keyword");
                    if !start_of_line(stream) {
                        return Phase::Block;
                    }
                    return self.push(Phase::Block, stream, unit);
                }
                _ => {}
            }
            if current == "return" {
                return self.push(Phase::Block, stream, 0);
            }
            if value == "variable" && is_placeholder(line) {
                return self.push(Phase::Block, stream, unit);
            }
        }
        self.context().phase
    }

    fn parens(&mut self, kind: Kind, stream: &StringStream<'_>, style: &mut Style) -> Phase {
        let unit = stream.indent_unit();
        let line = stream.line();
        match kind {
            Mark('(') => return self.push(Phase::Parens, stream, unit),
            Mark(')') => {
                if self
                    .contexts
                    .iter()
                    .rev()
                    .nth(1)
                    .is_some_and(|cx| cx.phase == Phase::Parens)
                {
                    return self.pop();
                }
                let word = first_word(line);
                let end = end_of_line(stream);
                let indent = if pattern!(r"^[a-z][\w-]*\(", "i").is_match(line) && end
                    || is_block(word)
                    || !pattern!(r#"^-?[a-z][\w.\[\]'"-]*\s*="#).is_match(line) && is_tag(word)
                {
                    unit
                } else if pattern!(r#"^[\$-]?[a-z][\w.\[\]'"-]*\s*="#).is_match(line)
                    || pattern!(r"^\s*[(\)0-9]").is_match(line)
                    || pattern!(r"^\s+[a-z][\w-]*\(", "i").is_match(line)
                    || pattern!(r"^\s+[\$-]?[a-z]", "i").is_match(line)
                {
                    0
                } else if end {
                    unit
                } else {
                    0
                };
                return self.push(Phase::Block, stream, indent);
            }
            Kind::Def if is_property(&stream.current()[1..]) => *style = Some("variable"),
            Kind::Word => {
                let word = stream.current();
                let value = word_as_value(word);
                *style = Some(if value == "property" || word == "to" {
                    "atom"
                } else if value == "tag" && is_tag_variable(word) {
                    "variable"
                } else {
                    value
                });
            }
            Kind::VariableName => return self.push(Phase::VariableName, stream, unit),
            _ => {}
        }
        if is_pseudo(kind, stream) {
            return self.push(Phase::Pseudo, stream, unit);
        }
        self.context().phase
    }

    fn at_block(&mut self, kind: Kind, stream: &StringStream<'_>, style: &mut Style) -> Phase {
        let unit = stream.indent_unit();
        if kind == Mark('(') {
            return self.push(Phase::AtBlockParens, stream, unit);
        }
        if is_block_token(kind, stream) {
            return self.push(Phase::Block, stream, unit);
        }
        if is_interpolation(kind, stream) {
            return self.push(Phase::Interpolation, stream, unit);
        }
        let word = stream.current();
        if kind == Kind::Word {
            let word_lower = word.to_ascii_lowercase();
            let key = word_lower.as_str();
            let value = if matches!(key, "only" | "not" | "and" | "or") {
                "keyword"
            } else if words::DOCUMENT_TYPES.contains(&key) {
                "tag"
            } else if words::MEDIA_TYPES.contains(&key) {
                "attribute"
            } else if words::MEDIA_FEATURES.contains(&key) {
                "property"
            } else if words::NONSTANDARD_PROPERTIES.contains(&key) {
                "string.special"
            } else {
                word_as_value(word)
            };
            *style = Some(value);
            if value == "tag" && end_of_line(stream) {
                return self.push(Phase::Block, stream, unit);
            }
        }
        if kind == Kind::Operator && matches!(word, "not" | "and" | "or") {
            *style = Some("keyword");
        }
        self.context().phase
    }

    fn keyframes(&mut self, kind: Kind, stream: &StringStream<'_>, style: &mut Style) -> Phase {
        let unit = stream.indent_unit();
        let word = stream.current();
        match kind {
            Mark('{' | '}') => {
                if kind == Mark('}') && start_of_line(stream) {
                    let indent = self.context().indent.saturating_sub(unit);
                    let phase = self.pop();
                    self.contexts.last_mut().unwrap().indent = indent;
                    return phase;
                }
                return self.push(Phase::Keyframes, stream, unit);
            }
            Kind::Unit if pattern!(r"^[0-9]+%$").is_match(word) => {
                return self.push(Phase::Keyframes, stream, unit);
            }
            Kind::Word => {
                let value = word_as_value(word);
                *style = Some(value);
                if value == "block-keyword" {
                    *style = Some("keyword");
                    return self.push(Phase::Keyframes, stream, unit);
                }
            }
            Kind::Def if is_at_block(word) => {
                let phase = if end_of_line(stream) {
                    Phase::Block
                } else {
                    Phase::AtBlock
                };
                return self.push(phase, stream, unit);
            }
            Kind::Mixin => return self.push(Phase::Block, stream, 0),
            _ => {}
        }
        self.context().phase
    }

    fn interpolation(&mut self, kind: Kind, stream: &StringStream<'_>, style: &mut Style) -> Phase {
        let unit = stream.indent_unit();
        let line = stream.line();
        match kind {
            Mark('{') => {
                self.pop();
                return self.push(Phase::Block, stream, unit);
            }
            Mark('}') => {
                let indent = if pattern!(r"^\s*[.#:\[*&>~+/]").is_match(line)
                    || pattern!(r"^\s*[a-z]", "i").is_match(line) && is_tag(first_word(line))
                {
                    unit
                } else if !pattern!(r"^(?:\{|\s*&)").is_match(line)
                    || pattern!(r"^\s*[\w-]").is_match(rest(stream))
                {
                    0
                } else {
                    unit
                };
                return self.push(Phase::Block, stream, indent);
            }
            Kind::VariableName => return self.push(Phase::VariableName, stream, 0),
            Kind::Word => {
                let value = word_as_value(stream.current());
                *style = Some(if value == "tag" { "atom" } else { value });
            }
            _ => {}
        }
        self.context().phase
    }
}

trait PatternTest {
    fn is_match(&self, text: &str) -> bool;
}

impl PatternTest for regress::Regex {
    fn is_match(&self, text: &str) -> bool {
        self.find(text).is_some()
    }
}

fn is_block_token(kind: Kind, stream: &StringStream<'_>) -> bool {
    kind == Kind::BlockMixin
        || end_of_line(stream) && matches!(kind, Mark('{' | ']') | Kind::Hash | Kind::Qualifier)
}

fn is_interpolation(kind: Kind, stream: &StringStream<'_>) -> bool {
    kind == Mark('{') && pattern!(r"^\s*\$?[\w-]+").is_match(rest(stream))
}

fn is_pseudo(kind: Kind, stream: &StringStream<'_>) -> bool {
    kind == Mark(':') && pattern!(r"^[a-z-]+").is_match(rest(stream))
}

fn is_at_block(word: &str) -> bool {
    pattern!(r"@(?:font-face|media|supports|(?:-moz-)?document)").is_match(word)
}

fn is_tag_variable(word: &str) -> bool {
    pattern!(r"^(?:a|b|i|s|col|em)$", "i").is_match(word)
}

fn is_placeholder(line: &str) -> bool {
    pattern!(r#"^\s?\$[\w.\[\]'"-]+$"#).is_match(line)
}

fn is_attribute(line: &str, word: &str) -> bool {
    use crate::legacy_modes::js_space;

    line.match_indices(word).any(|(start, matched)| {
        line[..start].trim_end_matches(js_space).ends_with('[')
            || line[start + matched.len()..]
                .trim_start_matches(js_space)
                .starts_with(']')
    })
}

fn word_as_value(word: &str) -> &'static str {
    let lower = word.to_ascii_lowercase();
    let key = lower.as_str();
    if is_tag(key) {
        "tag"
    } else if is_block(key) {
        "block-keyword"
    } else if is_property(key) {
        "property"
    } else if words::VALUES.contains(&key) || words::ATOMS.contains(&key) {
        "atom"
    } else if key == "return" || words::COLORS.contains(&key) {
        "keyword"
    } else if word.starts_with(|ch: char| ch.is_ascii_uppercase()) {
        "string"
    } else {
        "variable"
    }
}
