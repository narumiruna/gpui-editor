//! Native `mode/asn1` factory from `@codemirror/legacy-modes` 6.5.5.
//! See `docs/licenses/codemirror-legacy-modes.txt`.

use std::{borrow::Cow, collections::HashSet, sync::LazyLock};

use crate::{
    commands::CommentTokens,
    language::{LanguageData, StreamLanguage},
    stream_parser::{StreamParser, StringStream},
};

use super::js_space;

/// Replacement vocabularies for the ASN.1 factory. `None` uses the upstream
/// table. `Some` with an empty set disables that table.
#[derive(Default)]
pub struct Asn1Config {
    pub keywords: Option<HashSet<String>>,
    pub cmip_verbs: Option<HashSet<String>>,
    pub compare_types: Option<HashSet<String>>,
    pub status: Option<HashSet<String>>,
    pub tags: Option<HashSet<String>>,
    pub storage: Option<HashSet<String>>,
    pub modifier: Option<HashSet<String>>,
    pub access_types: Option<HashSet<String>>,
}

/// Build an ASN.1 stream parser. Like the pinned factory, quoted strings can
/// continue across lines and there is no indentation query. The original
/// factory's private statement contexts do not affect emitted tokens.
pub fn asn1(config: Asn1Config) -> Asn1 {
    let mut lists = [
        config.keywords,
        config.cmip_verbs,
        config.compare_types,
        config.status,
        config.tags,
        config.storage,
        config.modifier,
        config.access_types,
    ];
    Asn1(std::array::from_fn(|i| {
        lists[i].take().unwrap_or_else(|| {
            WORDS[i]
                .split_ascii_whitespace()
                .map(str::to_owned)
                .collect()
        })
    }))
}

pub struct Asn1([HashSet<String>; 8]);

#[derive(Clone, Debug, Default)]
pub struct Asn1State(Option<char>);

pub const LINE_COMMENT: &str = "--";

pub fn indent_on_input(prefix: &str) -> bool {
    matches!(prefix.trim_start_matches(js_space), "{" | "}")
}

pub fn language_data() -> LanguageData {
    LanguageData::default()
        .with("indentOnInput", indent_on_input as fn(&str) -> bool)
        .with(
            "commentTokens",
            CommentTokens {
                line: Some(LINE_COMMENT),
                block: None,
            },
        )
}

/// The concrete result of the catalog's `asn1({})` call.
pub fn default_language() -> &'static StreamLanguage<Asn1> {
    static LANGUAGE: LazyLock<StreamLanguage<Asn1>> = LazyLock::new(|| {
        StreamLanguage::new(asn1(Asn1Config::default())).with_language_data(language_data())
    });
    &LANGUAGE
}

impl StreamParser for Asn1 {
    type State = Asn1State;

    fn name(&self) -> Option<&str> {
        Some("asn1")
    }

    fn start_state(&self, _: usize) -> Asn1State {
        Asn1State::default()
    }

    fn token(&self, stream: &mut StringStream<'_>, state: &mut Asn1State) -> Option<Cow<'_, str>> {
        if stream.eat_while(js_space) {
            return None;
        }
        let style = if let Some(quote) = state.0 {
            string(stream, state, quote)
        } else {
            match stream.next()? {
                quote @ ('"' | '\'') => {
                    state.0 = Some(quote);
                    string(stream, state, quote)
                }
                '[' | ']' | '(' | ')' | '{' | '}' | ':' | '=' | ',' | ';' => "punctuation",
                '-' if stream.eat('-').is_some() => {
                    stream.skip_to_end();
                    "comment"
                }
                '0'..='9' => {
                    stream.eat_while(|ch: char| word(ch) || ch == '.');
                    "number"
                }
                '|' | '^' => {
                    stream.eat_while(|ch| matches!(ch, '|' | '^'));
                    "operator"
                }
                _ => {
                    stream.eat_while(|ch| word(ch) || ch == '-');
                    let token = stream.current();
                    self.0
                        .iter()
                        .zip(STYLES)
                        .find_map(|(table, style)| table.contains(token).then_some(style))
                        .unwrap_or("variableName")
                }
            }
        };
        Some(Cow::Borrowed(style))
    }
}

fn word(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

fn string(stream: &mut StringStream<'_>, state: &mut Asn1State, quote: char) -> &'static str {
    let mut escaped = false;
    while let Some(ch) = stream.next() {
        if ch == quote && !escaped {
            if matches!(stream.peek(), Some('b' | 'B' | 'h' | 'H' | 'o' | 'O')) {
                stream.next();
            }
            state.0 = None;
            break;
        }
        escaped = !escaped && ch == '\\';
    }
    "string"
}

const STYLES: [&str; 8] = [
    "keyword",
    "variableName",
    "atom",
    "comment",
    "typeName",
    "modifier",
    "modifier",
    "modifier",
];
const WORDS: [&str; 8] = [
    "DEFINITIONS OBJECTS IF DERIVED INFORMATION ACTION REPLY ANY NAMED CHARACTERIZED BEHAVIOUR REGISTERED WITH AS IDENTIFIED CONSTRAINED BY PRESENT BEGIN IMPORTS FROM UNITS SYNTAX MIN-ACCESS MAX-ACCESS MINACCESS MAXACCESS REVISION STATUS DESCRIPTION SEQUENCE SET COMPONENTS OF CHOICE DistinguishedName ENUMERATED SIZE MODULE END INDEX AUGMENTS EXTENSIBILITY IMPLIED EXPORTS",
    "ACTIONS ADD GET NOTIFICATIONS REPLACE REMOVE",
    "OPTIONAL DEFAULT MANAGED MODULE-TYPE MODULE_IDENTITY MODULE-COMPLIANCE OBJECT-TYPE OBJECT-IDENTITY OBJECT-COMPLIANCE MODE CONFIRMED CONDITIONAL SUBORDINATE SUPERIOR CLASS TRUE FALSE NULL TEXTUAL-CONVENTION",
    "current deprecated mandatory obsolete",
    "APPLICATION AUTOMATIC EXPLICIT IMPLICIT PRIVATE TAGS UNIVERSAL",
    "BOOLEAN INTEGER OBJECT IDENTIFIER BIT OCTET STRING UTCTime InterfaceIndex IANAifType CMIP-Attribute REAL PACKAGE PACKAGES IpAddress PhysAddress NetworkAddress BITS BMPString TimeStamp TimeTicks TruthValue RowStatus DisplayString GeneralString GraphicString IA5String NumericString PrintableString SnmpAdminString TeletexString UTF8String VideotexString VisibleString StringStore ISO646String T61String UniversalString Unsigned32 Integer32 Gauge Gauge32 Counter Counter32 Counter64",
    "ATTRIBUTE ATTRIBUTES MANDATORY-GROUP MANDATORY-GROUPS GROUP GROUPS ELEMENTS EQUALITY ORDERING SUBSTRINGS DEFINED",
    "not-accessible accessible-for-notify read-only read-create read-write",
];
