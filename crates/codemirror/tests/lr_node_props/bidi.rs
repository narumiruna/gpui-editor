use gpui_editor_codemirror::{
    EditError, Text,
    language::{BidiIsolateOptions, bidi_isolates},
    language_data::{cpp_language, rust_language},
    view::{Viewport, bidi::Direction},
};

use super::*;

#[test]
fn configured_lr_isolation_keeps_nested_ranges_directions_and_retained_snapshots() {
    let profiles: Vec<Value> = include_str!("../fixtures/lr_configured_bidi.jsonl")
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(profiles.len(), 4);
    for row in profiles {
        let props = row["props"].as_object().unwrap().clone();
        let sources = [ISOLATE.add(move |ty| {
            Some(match props.get(ty.name())? {
                Value::Null => None,
                value => Some(match value.as_str().unwrap() {
                    "auto" => Isolation::Auto,
                    "rtl" => Isolation::Rtl,
                    "ltr" => Isolation::Ltr,
                    _ => unreachable!(),
                }),
            })
        })];
        let (stock, configured) = match row["name"].as_str().unwrap() {
            "rust" => (
                rust_language().extension(),
                rust_language().with_node_props(&sources).extension(),
            ),
            "cpp" => (
                cpp_language().extension(),
                cpp_language().with_node_props(&sources).extension(),
            ),
            _ => {
                let [stock, _] = handles(&row);
                let configured = with_props(&stock, &sources);
                (
                    with_lr!(&stock, handle => handle.extension()),
                    with_lr!(&configured, handle => handle.extension()),
                )
            }
        };
        let cases = row["cases"].as_array().unwrap();
        assert_eq!(cases.len(), 7);
        for case in cases {
            let input = case["input"].as_str().unwrap();
            let always_isolate = case["always"].as_bool().unwrap();
            let options = BidiIsolateOptions {
                always_isolate,
                ..Default::default()
            };
            let to_marks =
                |snapshot: &SupportedDocument, viewport: &Viewport| match snapshot.syntax() {
                    NativeSyntax::Cpp(doc) => doc.bidi_isolate_marks(viewport, options),
                    NativeSyntax::Rust(doc) => doc.bidi_isolate_marks(viewport, options),
                    NativeSyntax::Json(doc) => doc.bidi_isolate_marks(viewport, options),
                    NativeSyntax::LezerGrammar(doc) => doc.bidi_isolate_marks(viewport, options),
                    _ => unreachable!(),
                };
            let records = |snapshot: &SupportedDocument, viewport: &Viewport| {
                let marks = to_marks(snapshot, viewport).unwrap();
                marks
                    .iter(0)
                    .map(|mark| {
                        assert_eq!(mark.value.class(), "cm-iso");
                        let direction = match mark.value.bidi_direction() {
                            None => "auto",
                            Some(Direction::Ltr) => "ltr",
                            Some(Direction::Rtl) => "rtl",
                        };
                        json!([mark.from, mark.to, direction])
                    })
                    .collect::<Vec<_>>()
            };
            let slot = Compartment::new();
            let mut state = EditorState::with_extensions(
                input,
                Extension::group([slot.of(stock.clone()), bidi_isolates(always_isolate)]),
            )
            .unwrap();
            let before = attached_language_snapshot(&state).unwrap();
            let whole = Viewport::complete(before.text());
            let before_marks = records(&before, &whole);
            if case["switched"].as_bool().unwrap() {
                state
                    .update(
                        Transaction::default().with_effects([slot.reconfigure(configured.clone())]),
                    )
                    .unwrap();
            }
            if let Some(edit) = case["edit"].as_array() {
                let at = |n: usize| usize::try_from(edit[n].as_u64().unwrap()).unwrap();
                let change =
                    ChangeSet::single(Change::new(at(0)..at(1), edit[2].as_str().unwrap()))
                        .unwrap();
                state.update(Transaction::new(change)).unwrap();
            }
            let snapshot = attached_language_snapshot(&state).unwrap();
            assert_eq!(json!(snapshot.text().as_str()), case["updated"]);
            let visible: Vec<[usize; 2]> = serde_json::from_value(case["visible"].clone()).unwrap();
            let viewport = Viewport::new(
                snapshot.text(),
                0..snapshot.text().len(),
                visible.into_iter().map(|[from, to]| from..to),
            )
            .unwrap();
            assert_eq!(
                json!(records(&snapshot, &viewport)),
                case["ranges"],
                "{}: {}",
                row["name"],
                case["step"]
            );
            assert_eq!(records(&before, &whole), before_marks);
            let inside = snapshot.text().as_str().find('😀').unwrap() + 1;
            let foreign = Text::new(" ".repeat(snapshot.text().len()));
            let invalid = Viewport::new(
                &foreign,
                0..foreign.len(),
                std::iter::once(inside..inside + 1),
            )
            .unwrap();
            assert_eq!(
                to_marks(&snapshot, &invalid).unwrap_err(),
                EditError::InvalidRange
            );
        }
    }
}
