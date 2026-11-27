//! Native `mkXML` factory from `@codemirror/legacy-modes` 6.5.5.
//! See `docs/licenses/codemirror-legacy-modes.txt` for the upstream MIT notice.

use std::{
    borrow::Cow,
    collections::{HashMap, HashSet},
    sync::Arc,
};

use super::{LegacyMode, data, xml};
use crate::{
    language::StreamLanguage,
    stream_parser::{StreamParser, StringStream},
};

/// Options for [`mk_xml`]. Set `html_mode` to select HTML defaults. A table
/// replaces the default table, even when empty. The scanner lowercases tag
/// names for all table lookups except `do_not_indent`. Supply keys in the
/// form that the scanner looks up. Closing tag matching is case-sensitive.
/// Upstream's `caseFold` option is not read and has no Rust counterpart.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LegacyXmlConfig {
    pub html_mode: bool,
    /// Tags that end without an explicit closing tag.
    pub auto_self_closers: Option<HashSet<String>>,
    /// Tags that can end when a different closing tag appears.
    pub implicitly_closed: Option<HashSet<String>>,
    /// Map from a parent tag to the opening tags that end that parent.
    pub context_grabbers: Option<HashMap<String, HashSet<String>>>,
    /// Tags that disable indentation for their contents.
    pub do_not_indent: Option<HashSet<String>>,
    /// Allow unquoted attribute values. By default, only HTML mode allows them.
    pub allow_unquoted: Option<bool>,
    /// Allow attributes without values. By default, only HTML mode allows them.
    pub allow_missing: Option<bool>,
    /// Allow `<>` and `</>`.
    pub allow_missing_tag_name: bool,
    /// Require a closing tag name to match its opening tag name. Default: true.
    pub match_closing: bool,
    /// Align new attribute names after the opening tag name. Default: true.
    pub multiline_tag_indent_past_tag: bool,
    /// Number of indent units for attributes when indenting from the opening
    /// bracket instead of past the name. Zero also means one unit.
    pub multiline_tag_indent_factor: usize,
    /// Align a line that contains `<![CDATA[` to column zero in text or tag context.
    pub align_cdata: bool,
}

impl Default for LegacyXmlConfig {
    fn default() -> Self {
        Self::XML
    }
}

impl LegacyXmlConfig {
    pub(super) const XML: Self = Self {
        html_mode: false,
        auto_self_closers: None,
        implicitly_closed: None,
        context_grabbers: None,
        do_not_indent: None,
        allow_unquoted: None,
        allow_missing: None,
        allow_missing_tag_name: false,
        match_closing: true,
        multiline_tag_indent_past_tag: true,
        multiline_tag_indent_factor: 1,
        align_cdata: false,
    };

    pub(super) const HTML: Self = Self {
        html_mode: true,
        ..Self::XML
    };

    pub(super) fn allows_unquoted(&self) -> bool {
        self.allow_unquoted.unwrap_or(self.html_mode)
    }

    pub(super) fn allows_missing(&self) -> bool {
        self.allow_missing.unwrap_or(self.html_mode)
    }

    pub(super) fn auto_closes(&self, tag: &str) -> bool {
        match &self.auto_self_closers {
            Some(tags) => tags.contains(&tag.to_lowercase()),
            None => self.html_mode && xml::html_auto_closes(&tag.to_lowercase()),
        }
    }

    pub(super) fn implicit(&self, tag: &str) -> bool {
        match &self.implicitly_closed {
            Some(tags) => tags.contains(&tag.to_lowercase()),
            None => self.html_mode && xml::html_implicit(&tag.to_lowercase()),
        }
    }

    pub(super) fn grabs(&self, parent: &str, next: &str) -> bool {
        match &self.context_grabbers {
            Some(grabbers) => grabbers
                .get(&parent.to_lowercase())
                .is_some_and(|tags| tags.contains(&next.to_lowercase())),
            None => self.html_mode && xml::html_grabs(&parent.to_lowercase(), &next.to_lowercase()),
        }
    }

    pub(super) fn no_indent(&self, tag: &str) -> bool {
        match &self.do_not_indent {
            Some(tags) => tags.contains(tag),
            None => self.html_mode && tag == "pre",
        }
    }
}

/// Build a configurable legacy XML stream parser. The HTML defaults do not
/// nest other modes. This is separate from `@codemirror/lang-xml`.
///
/// Token ranges use UTF-8 bytes, and indentation counts Unicode scalars, not
/// UTF-16 code units. The typed options do not emulate JavaScript coercions
/// or object prototypes. Parser state is opaque: direct mutation of upstream
/// state fields such as `baseIndent` is not supported.
pub fn mk_xml(config: LegacyXmlConfig) -> LegacyXml {
    LegacyXml(Arc::new(config))
}

/// The native configurable legacy XML parser.
#[derive(Clone, Debug)]
pub struct LegacyXml(Arc<LegacyXmlConfig>);

impl LegacyXml {
    /// Get the upstream `configuration` value. The parser name is always `xml`.
    pub fn configuration(&self) -> &'static str {
        if self.0.html_mode { "html" } else { "xml" }
    }

    /// Get a stream language with XML comment markers and the indentation trigger.
    pub fn language(&self) -> StreamLanguage<Self> {
        StreamLanguage::new(self.clone()).with_language_data(data(LegacyMode::Xml))
    }
}

/// A checkpoint in a legacy XML stream. Use it with the parser that created it.
#[derive(Clone, Debug)]
pub struct LegacyXmlState(xml::Scanner);

impl LegacyXmlState {
    /// Stop waiting for an attribute value after `=`. Otherwise, do nothing.
    pub fn skip_attribute(&mut self) {
        self.0.skip_attribute();
    }

    /// Get the name of an opening tag that has started but not ended. The
    /// upstream `close` field is always false; no closing tag name is stored.
    pub fn xml_current_tag(&self) -> Option<&str> {
        self.0.current_tag()
    }

    /// Read the open tag names in order from outermost to innermost. An
    /// unnamed open tag has an empty name.
    pub fn xml_current_context(&self) -> impl Iterator<Item = &str> {
        self.0.current_context()
    }
}

impl StreamParser for LegacyXml {
    type State = LegacyXmlState;

    fn name(&self) -> Option<&str> {
        Some("xml")
    }

    fn start_state(&self, _indent_unit: usize) -> Self::State {
        LegacyXmlState(xml::Scanner::default())
    }

    fn token(
        &self,
        stream: &mut StringStream<'_>,
        state: &mut Self::State,
    ) -> Option<Cow<'_, str>> {
        state.0.token(&self.0, stream).map(Cow::Borrowed)
    }

    fn indent(&self, state: &Self::State, text_after: &str, indent_unit: usize) -> Option<usize> {
        state.0.indent(&self.0, text_after, indent_unit)
    }
}
