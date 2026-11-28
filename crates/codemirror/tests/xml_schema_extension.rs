use std::sync::Arc;

use gpui_editor_codemirror::{
    Assoc, Compartment, EditorState, Extension, Text, Transaction,
    autocomplete::{Completion, session::SourceResult},
    language::xml::{
        NativeXmlSchemaExtension, XmlAttrSpec, XmlAttribute, XmlDocument, XmlElementSpec,
        XmlSchema, XmlValue, xml_completion_source,
    },
    language_data::{XmlCompletionSource, xml_extension},
};
use serde_json::{Value, json};

fn oracle_schema() -> Extension {
    with_oracle_schema(|schema| NativeXmlSchemaExtension::new(schema).extension())
}

fn with_oracle_schema(build: impl FnOnce(XmlSchema<'_>) -> Extension) -> Extension {
    // The extension must outlive the caller's schema, including its strings.
    let root = String::from("root");
    let override_insert = String::from("\"custom\"");
    build(XmlSchema {
        elements: &[
            XmlElementSpec {
                name: &root,
                top: true,
                children: Some(&["zeta", "unknown", "alpha"]),
                text_content: &["yes", "no"],
                completion: Some(Completion {
                    detail: Some("Root"),
                    boost: 1,
                    ..Completion::new("")
                }),
                attributes: &[
                    XmlAttribute::Named("shared"),
                    XmlAttribute::Named("undeclared"),
                    XmlAttribute::Inline(XmlAttrSpec {
                        name: "mode",
                        values: Some(&[
                            XmlValue::Text("one"),
                            XmlValue::Completion(Completion {
                                detail: Some("Override"),
                                insert: Some(&override_insert),
                                ..Completion::new("okay")
                            }),
                            XmlValue::Completion(Completion {
                                kind: Some("enum"),
                                ..Completion::new("\"open\"")
                            }),
                        ]),
                        ..Default::default()
                    }),
                ],
            },
            XmlElementSpec {
                name: "alpha",
                children: Some(&[]),
                ..Default::default()
            },
            XmlElementSpec {
                name: "zeta",
                ..Default::default()
            },
            XmlElementSpec {
                name: "envelope",
                top: true,
                ..Default::default()
            },
        ],
        attributes: &[
            XmlAttrSpec {
                name: "shared",
                values: Some(&[XmlValue::Text("shared value")]),
                completion: Some(Completion {
                    detail: Some("Shared"),
                    ..Completion::new("")
                }),
                ..Default::default()
            },
            XmlAttrSpec {
                name: "z-global",
                global: true,
                ..Default::default()
            },
            XmlAttrSpec {
                name: "a-global",
                global: true,
                ..Default::default()
            },
            XmlAttrSpec {
                name: "mode",
                values: Some(&[XmlValue::Text("fallback")]),
                ..Default::default()
            },
        ],
    })
}

fn summarize(result: Option<SourceResult>) -> Value {
    result.map_or(Value::Null, |result| {
        json!({
            "from": result.from,
            "to": result.to,
            "options": result.options.iter().map(|item| json!({
                "label": item.label, "type": item.kind, "detail": item.detail,
                "boost": item.boost, "apply": item.insert,
            })).collect::<Vec<_>>(),
        })
    })
}

#[test]
fn configured_headless_sources_match_pinned_schema_without_entities() {
    let configured = with_oracle_schema(|schema| xml_extension(schema, true));
    let stock = xml_extension(XmlSchema::default(), true);
    for row in include_str!("fixtures/xml_schema_completion.jsonl").lines() {
        let row: Value = serde_json::from_str(row).unwrap();
        let marked = row["marked"].as_str().unwrap();
        let at = marked.find('|').unwrap();
        let explicit = row["explicit"].as_bool().unwrap();
        let extension = if row["enabled"] == true {
            &configured
        } else {
            &stock
        };
        let state =
            EditorState::with_extensions(marked.replacen('|', "", 1), extension.clone()).unwrap();
        let sources =
            state.language_data_at::<XmlCompletionSource>("autocomplete", at, Assoc::Before);
        assert_eq!(sources.len(), 1);
        assert_eq!(
            summarize(sources[0](&state, at, explicit)),
            row["result"],
            "{marked}"
        );
        assert!(NativeXmlSchemaExtension::source_for(&state).is_none());
    }
}

#[test]
fn registered_and_stock_sources_match_pinned_order_ranges_and_metadata() {
    let state = EditorState::create("", None, oracle_schema()).unwrap();
    let configured = NativeXmlSchemaExtension::source_for(&state).unwrap();
    let empty = EditorState::new("");
    assert!(NativeXmlSchemaExtension::source_for(&empty).is_none());
    for row in include_str!("fixtures/xml_schema_completion.jsonl").lines() {
        let row: Value = serde_json::from_str(row).unwrap();
        let marked = row["marked"].as_str().unwrap();
        let explicit = row["explicit"].as_bool().unwrap();
        let pos = marked.find('|').unwrap();
        let doc = XmlDocument::parse(Text::new(marked.replacen('|', "", 1)));
        let result = if row["enabled"] == true {
            configured.complete(&doc, pos, explicit)
        } else {
            xml_completion_source(&doc, pos, explicit, XmlSchema::default())
        };
        if marked == "<root>&a|</root>" {
            // The pinned source has no entity completion. Keep the native stock addition.
            assert!(row["result"].is_null());
            assert!(
                result
                    .unwrap()
                    .options
                    .iter()
                    .any(|item| item.label == "&amp;")
            );
            continue;
        }
        assert_eq!(summarize(result), row["result"], "{marked}");
    }
}

#[test]
fn winning_registration_retains_identity_until_replaced_or_removed() {
    let compartment = Compartment::new();
    let registration = oracle_schema();
    let fallback = Compartment::new();
    let mut state = EditorState::create(
        "",
        None,
        Extension::group([
            compartment.of(registration.clone()),
            fallback.of(oracle_schema()),
        ]),
    )
    .unwrap();
    let first = NativeXmlSchemaExtension::source_for(&state).unwrap();
    let same = NativeXmlSchemaExtension::source_for(&state.clone()).unwrap();
    assert!(Arc::ptr_eq(&first, &same));

    state
        .update(Transaction::default().with_effects([fallback.reconfigure(oracle_schema())]))
        .unwrap();
    assert!(Arc::ptr_eq(
        &first,
        &NativeXmlSchemaExtension::source_for(&state).unwrap()
    ));

    state
        .update(
            Transaction::default().with_effects([compartment.reconfigure(Extension::default())]),
        )
        .unwrap();
    let second = NativeXmlSchemaExtension::source_for(&state).unwrap();
    assert!(!Arc::ptr_eq(&first, &second));
    state
        .update(Transaction::default().with_effects([compartment.reconfigure(registration)]))
        .unwrap();
    let restored = NativeXmlSchemaExtension::source_for(&state).unwrap();
    assert!(!Arc::ptr_eq(&first, &restored));
    assert!(!Arc::ptr_eq(&second, &restored));

    state
        .update(Transaction::default().with_effects([Extension::default().reconfigure()]))
        .unwrap();
    assert!(NativeXmlSchemaExtension::source_for(&state).is_none());
}
