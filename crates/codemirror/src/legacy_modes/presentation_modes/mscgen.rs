use super::{advance, remaining, starts_ascii_case, word_prefix};
use crate::{legacy_modes::LegacyMode, stream_parser::StringStream};

#[derive(Clone, Debug, Default)]
pub(super) struct State {
    in_comment: bool,
    in_string: bool,
    in_attributes: bool,
}

impl State {
    pub(super) fn token(
        &mut self,
        mode: LegacyMode,
        stream: &mut StringStream<'_>,
    ) -> Option<&'static str> {
        // Braces take priority even at the start of a comment or string line.
        if stream.eat_if(|ch| matches!(ch, '{' | '}')).is_some() {
            return Some("bracket");
        }
        if self.in_comment {
            let rest = remaining(stream);
            let suffix = rest.trim_start_matches(comment_text);
            if let Some(after) = suffix.strip_prefix("*/") {
                advance(stream, rest.len() - after.len());
                self.in_comment = false;
            } else {
                stream.skip_to_end();
            }
            return Some("comment");
        }
        if stream.match_str("/*", true, false) {
            stream.eat_while(comment_text);
            self.in_comment = true;
            return Some("comment");
        }
        if stream.match_str("//", true, false) || stream.eat('#').is_some() {
            stream.skip_to_end();
            return Some("comment");
        }
        if self.in_string {
            stream.eat_while(|ch| ch != '"');
            self.in_string = stream.eat('"').is_none();
            return Some("string");
        }
        if stream.eat('"').is_some() {
            while stream.peek().is_some_and(|ch| ch != '"') {
                if stream.next() == Some('\\') {
                    stream.eat('"');
                }
            }
            self.in_string = true;
            return Some("string");
        }

        let rest = remaining(stream);
        let keywords = match mode {
            LegacyMode::Mscgen => &MSC_KEYWORDS[..1],
            LegacyMode::Xu => MSC_KEYWORDS,
            _ => &[],
        };
        let extra = if mode == LegacyMode::Mscgen {
            &[]
        } else {
            EXTENDED_KEYWORDS
        };
        if let Some(keyword) = keywords
            .iter()
            .chain(KEYWORDS)
            .chain(extra)
            .find(|keyword| word_prefix(rest, keyword))
        {
            advance(stream, keyword.len());
            return Some("keyword");
        }
        if let Some(arc) = ARCS.iter().find(|arc| starts_ascii_case(rest, arc)) {
            advance(stream, arc.len());
            return Some("keyword");
        }
        if stream.eat('=').is_some() {
            return Some("operator");
        }
        if let Some(constant) = CONSTANTS.iter().find(|&&constant| {
            (constant != "auto" || mode != LegacyMode::Mscgen) && starts_ascii_case(rest, constant)
        }) {
            advance(stream, constant.len());
            return Some("variable");
        }
        if mode != LegacyMode::Msgenny {
            if !self.in_attributes && stream.eat('[').is_some() {
                self.in_attributes = true;
                return Some("bracket");
            }
            if self.in_attributes {
                let extra = if mode == LegacyMode::Xu {
                    XU_ATTRIBUTES
                } else {
                    &[]
                };
                if let Some(attribute) = ATTRIBUTES
                    .iter()
                    .chain(extra)
                    .find(|attribute| word_prefix(rest, attribute))
                {
                    advance(stream, attribute.len());
                    return Some("attribute");
                }
                if stream.eat(']').is_some() {
                    self.in_attributes = false;
                    return Some("bracket");
                }
            }
        }
        stream.next();
        None
    }
}

fn comment_text(ch: char) -> bool {
    !matches!(ch, '*' | '/')
}

const MSC_KEYWORDS: &[&str] = &["msc", "xu"];
const KEYWORDS: &[&str] = &[
    "hscale",
    "width",
    "arcgradient",
    "wordwraparcs",
    "note",
    "abox",
    "rbox",
    "box",
];
const EXTENDED_KEYWORDS: &[&str] = &[
    "wordwrapentities",
    "watermark",
    "alt",
    "else",
    "opt",
    "break",
    "par",
    "seq",
    "strict",
    "neg",
    "critical",
    "ignore",
    "consider",
    "assert",
    "loop",
    "ref",
    "exc",
];
const ATTRIBUTES: &[&str] = &[
    "label",
    "idurl",
    "id",
    "url",
    "linecolor",
    "linecolour",
    "textcolor",
    "textcolour",
    "textbgcolor",
    "textbgcolour",
    "arclinecolor",
    "arclinecolour",
    "arctextcolor",
    "arctextcolour",
    "arctextbgcolor",
    "arctextbgcolour",
    "arcskip",
];
const XU_ATTRIBUTES: &[&str] = &["title", "deactivate", "activate", "activation"];

const ARCS: &[&str] = &[
    "|||", "...", "---", "--", "<->", "==", "<<=>>", "<=>", "..", "<<>>", "::", "<:>", "->", "=>>",
    "=>", ">>", ":>", "<-", "<<=", "<=", "<<", "<:", "x-", "-x",
];
const CONSTANTS: &[&str] = &["true", "false", "on", "off", "auto"];
