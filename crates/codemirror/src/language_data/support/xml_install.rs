use std::sync::{Arc, LazyLock};

use crate::{
    Assoc, EditorState, Extension, Transaction,
    autocomplete::session::SourceResult,
    language::{
        LanguageData,
        xml::{self, NativeXmlSchemaExtension, XmlSchema},
    },
    view::input::input_completion_handler,
};

use super::{
    LrDataFacet, NativeLanguage, NativeSyntax, attached_language_snapshot,
    input::close_switched_end_tags, metadata,
};

/// A callable XML `autocomplete` value. Positions use original UTF-8 bytes.
pub type XmlCompletionSource =
    Arc<dyn Fn(&EditorState, usize, bool) -> Option<SourceResult> + Send + Sync>;

static XML_DATA: LazyLock<LrDataFacet> =
    LazyLock::new(|| LrDataFacet::new(Some(metadata(NativeLanguage::Xml))));
static XML_ROOT: LazyLock<Extension> =
    LazyLock::new(|| NativeLanguage::Xml.support().tracked_syntax_extension());

pub(super) fn configured_data(state: &EditorState) -> Vec<Arc<LanguageData>> {
    XML_DATA.read(state)
}

pub(super) fn extension() -> Extension {
    static BUNDLE: LazyLock<Extension> =
        LazyLock::new(|| xml_extension(XmlSchema::default(), true));
    BUNDLE.clone()
}

/// Install the stock XML parser and a schema completion source for a headless
/// view. The extension owns a copy of the schema. Pass `false` to omit tag
/// auto-close. Use [`NativeXmlSchemaExtension`] for GPUI schema registration.
pub fn xml_extension(schema: XmlSchema<'_>, auto_close_tags: bool) -> Extension {
    let schema = NativeXmlSchemaExtension::new(schema);
    let source: XmlCompletionSource = Arc::new(move |state, at, explicit| {
        let snapshot = attached_language_snapshot(state)?;
        let NativeSyntax::Xml(doc) = snapshot.syntax() else {
            return None;
        };
        schema.with_schema(|schema| xml::schema_completion_source(doc, at, explicit, schema))
    });
    let mut extensions = vec![
        XML_ROOT.clone(),
        XML_DATA.of(LanguageData::default().with("autocomplete", source)),
    ];
    if auto_close_tags {
        extensions.push(close_tags_extension());
    }
    Extension::group(extensions)
}

fn close_tags_extension() -> Extension {
    static CLOSE_TAGS: LazyLock<Extension> = LazyLock::new(|| {
        input_completion_handler(|input, default| {
            let start = input.state();
            let at = input.range();
            let typed = input.text();
            if start.read_only() || !at.is_empty() || !matches!(typed, ">" | "/") {
                return Ok(None);
            }
            if attached_language_snapshot(start)
                .and_then(|doc| doc.language_at(at.start, Assoc::Before))
                != Some(NativeLanguage::Xml)
            {
                return Ok(None);
            }
            let state = &default()?.state;
            let Some(snapshot) = attached_language_snapshot(state) else {
                return Ok(None);
            };
            match snapshot.syntax() {
                NativeSyntax::Xml(doc) => {
                    Ok(
                        xml::close_tags_after(state, doc, 0, typed, Some(Assoc::Before))?
                            .map(Transaction::with_scroll_into_view),
                    )
                }
                syntax @ NativeSyntax::Html(_) if typed == ">" => {
                    close_switched_end_tags(state, syntax)
                }
                _ => Ok(None),
            }
        })
    });
    CLOSE_TAGS.clone()
}
