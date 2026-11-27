use gpui::{Modifiers, TestAppContext};
use gpui_editor_codemirror::{
    Change, ChangeSet, Compartment, Extension, StateEffect, Transaction,
    autocomplete::session::CompletionItem,
    language::sql::{SqlDialect, SqlNamespace, SqlSchemaConfig},
    transaction_filter,
};

use super::{tests::activate_draft, *};
use crate::editor::bind_keys;

fn schema_extension(dialect: SqlDialect, schema: &SqlNamespace) -> Extension {
    NativeSqlSchemaExtension::new(
        dialect,
        SqlSchemaConfig {
            schema: Some(schema),
            ..Default::default()
        },
    )
    .extension()
}

fn configure(editor: &mut Editor, effect: StateEffect, cx: &mut Context<Editor>) {
    let active = editor.active.clone();
    editor
        .view
        .update(
            Transaction::default().with_effects([effect]),
            active.clone(),
            |_| (None, active),
        )
        .unwrap();
    cx.notify();
}

#[gpui::test]
fn only_matching_sql_registrations_precede_the_stock_keyword_source(cx: &mut TestAppContext) {
    let draft = "```sql\r\nsel\r\n```\r\n\r\nOutside.";
    let head = draft.find("sel").unwrap() + 3;
    let mut item = CompletionItem::new("select");
    item.kind = Some("type".into());
    item.boost = -1; // Tie the keyword so the source order decides.
    let schema = SqlNamespace::Completions(vec![item.into()]);
    for dialect in [SqlDialect::PostgreSql, SqlDialect::StandardSql] {
        let extension = schema_extension(dialect, &schema);
        let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(draft.into(), window, cx));
        let executor = cx.update(|window, cx| {
            bind_keys(cx);
            editor.update(cx, |editor, cx| {
                configure(editor, extension.append(), cx);
                editor.move_to(head, false, cx);
                assert!(!editor.request_source_completion(true, cx));
                activate_draft(editor, head, window, cx);
            });
            cx.background_executor().clone()
        });
        cx.simulate_keystrokes("ctrl-space");
        executor.advance_clock(Duration::from_millis(50));
        cx.run_until_parked();
        cx.update(|_, cx| {
            let current = editor.read(cx).current_source_completion().unwrap();
            let options = current.session.options().unwrap();
            let matches: Vec<_> = options
                .iter()
                .filter(|option| option.item.label == "select")
                .map(|option| {
                    (
                        option.id.source(),
                        option.item.kind.as_deref(),
                        option.score,
                    )
                })
                .collect();
            assert_eq!(matches[0].0, SOURCE);
            if dialect == SqlDialect::StandardSql {
                assert_eq!(matches.len(), 2);
                assert_eq!(matches[0].1, Some("type"));
                assert_eq!(matches[1], (SECOND_SOURCE, Some("keyword"), matches[0].2));
            } else {
                assert_eq!(matches.len(), 1);
                assert_eq!(matches[0].1, Some("keyword"));
            }
        });
    }
}

#[gpui::test]
fn typed_qualified_sql_maps_the_quoted_range_and_undo_preserves_markdown(cx: &mut TestAppContext) {
    use SqlDialect::*;

    let schema = SqlNamespace::Namespaces(vec![(
        "acct.items".into(),
        SqlNamespace::Completions(vec!["order number".into(), "organization".into()]),
    )]);
    for (fence, dialect, quote) in [
        ("sql", StandardSql, '"'),
        ("postgresql", PostgreSql, '"'),
        ("mysql", MySql, '`'),
        ("sqlite", Sqlite, '`'),
        ("cql", Cassandra, '"'),
    ] {
        let draft = format!(
            "前\r\n\r\n```{fence} info\r\n-- 😀\r\nselect acct.items.{quote}o{quote}\r\n```\r\n\r\n後"
        );
        let quoted = draft.find(&format!("{quote}o")).unwrap();
        let head = quoted + 2;
        let split_crlf = draft.find("😀\r\n").unwrap() + "😀\r".len();
        let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(draft.clone(), window, cx));
        let executor = cx.update(|window, cx| {
            bind_keys(cx);
            editor.update(cx, |editor, cx| {
                configure(editor, schema_extension(dialect, &schema).append(), cx);
                activate_draft(editor, split_crlf, window, cx);
                assert_eq!(editor.cursor(), split_crlf);
                assert!(!editor.request_source_completion(true, cx));
                editor.move_to(head, false, cx);
            });
            cx.background_executor().clone()
        });
        cx.run_until_parked();
        cx.simulate_input("r");
        executor.advance_clock(Duration::from_millis(99));
        cx.update(|_, cx| {
            let current = editor.read(cx).current_source_completion().unwrap();
            assert_eq!(current.session.status(), CompletionStatus::Pending);
            assert!(current.session.options().unwrap().is_empty());
        });
        executor.advance_clock(Duration::from_millis(1));
        cx.run_until_parked();
        let label = format!("{quote}organization{quote}");
        let id = cx.update(|_, cx| {
            let current = editor.read(cx).current_source_completion().unwrap();
            let options = current.session.options().unwrap();
            assert!(options.iter().all(|option| option.id.source() == SOURCE));
            let option = options
                .iter()
                .find(|option| option.item.label == label)
                .unwrap();
            assert_eq!(option.range, quoted..head + 2); // Replacement includes the existing quote.
            option.id
        });
        // Type while the list is open. CodeMirror's valid span keeps the same option.
        cx.simulate_input("g");
        executor.advance_clock(Duration::from_millis(75));
        let typed = draft.replace(&format!("{quote}o{quote}"), &format!("{quote}org{quote}"));
        cx.update(|_, cx| {
            let editor = editor.read(cx);
            assert_eq!(editor.source(), typed);
            let session = &editor.current_source_completion().unwrap().session;
            let option = session.selected_completion().unwrap().unwrap();
            assert_eq!(option.id, id);
            assert_eq!(option.range, quoted..head + 3);
        });
        cx.simulate_keystrokes("enter");
        cx.update(|_, cx| {
            let editor = editor.read(cx);
            assert_eq!(
                editor.source(),
                draft.replace(&format!("{quote}o{quote}"), &label)
            );
            assert_eq!(editor.cursor(), quoted + label.len());
        });
        cx.simulate_keystrokes("cmd-z");
        cx.update(|_, cx| assert_eq!(editor.read(cx).source(), typed));
    }
}

#[gpui::test]
fn replacing_or_removing_the_schema_discards_the_popup_and_pending_queries(
    cx: &mut TestAppContext,
) {
    let draft = "```sql\r\nsel\r\n```\r\n\r\nOutside.";
    let head = draft.find("sel").unwrap() + 3;
    let compartment = Compartment::new();
    let old = schema_extension(
        SqlDialect::StandardSql,
        &SqlNamespace::Completions(vec!["select_old".into()]),
    );
    let new = schema_extension(
        SqlDialect::StandardSql,
        &SqlNamespace::Completions(vec!["select_new".into()]),
    );
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(draft.into(), window, cx));
    let executor = cx.update(|window, cx| {
        bind_keys(cx);
        editor.update(cx, |editor, cx| {
            activate_draft(editor, head, window, cx);
            configure(editor, compartment.of(old.clone()).append(), cx);
        });
        cx.background_executor().clone()
    });
    cx.simulate_keystrokes("ctrl-space");
    executor.advance_clock(Duration::from_millis(50));
    cx.run_until_parked();
    assert!(cx.debug_bounds("source-completion: select_old").is_some());
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            let option = editor
                .current_source_completion()
                .unwrap()
                .session
                .options()
                .unwrap()
                .into_iter()
                .find(|option| option.item.label == "select_old")
                .unwrap()
                .id;
            configure(editor, compartment.reconfigure(new), cx);
            assert!(editor.current_source_completion().is_none());
            assert!(!editor.accept_source_completion(Some(option), cx));
            assert!(editor.request_source_completion(true, cx));
        });
    });
    executor.advance_clock(Duration::from_millis(50));
    cx.run_until_parked();
    assert!(cx.debug_bounds("source-completion: select_old").is_none());
    assert!(cx.debug_bounds("source-completion: select_new").is_some());
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            assert!(editor.request_source_completion(true, cx));
            // Start the query in the same UI update that removes its registration.
            editor.completion.as_mut().unwrap().timer = None;
        });
    });
    executor.advance_clock(Duration::from_millis(50));
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            editor.run_source_completion_timer(cx);
            assert_eq!(
                editor
                    .current_source_completion()
                    .unwrap()
                    .session
                    .deadline(),
                None
            );
            configure(editor, compartment.reconfigure(Extension::default()), cx);
            assert!(editor.current_source_completion().is_none());
            assert!(editor.request_source_completion(true, cx));
        });
    });
    cx.run_until_parked();
    executor.advance_clock(Duration::from_millis(49));
    cx.update(|_, cx| {
        let current = editor.read(cx).current_source_completion().unwrap();
        assert_eq!(current.session.status(), CompletionStatus::Pending);
        assert!(current.session.options().unwrap().is_empty());
    });
    executor.advance_clock(Duration::from_millis(1));
    cx.run_until_parked();
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            let options = editor
                .current_source_completion()
                .unwrap()
                .session
                .options()
                .unwrap();
            assert!(options.iter().any(|option| option.item.label == "select"));
            assert!(
                options
                    .iter()
                    .all(|option| option.item.kind.as_deref() != Some("property"))
            );
            assert_eq!(editor.source(), draft);
            assert!(!editor.view.can_undo());

            configure(editor, compartment.reconfigure(old), cx);
            assert!(editor.request_source_completion(true, cx));
            configure(editor, compartment.reconfigure(Extension::default()), cx);
        });
    });
    executor.advance_clock(Duration::from_millis(50));
    cx.run_until_parked();
    cx.update(|_, cx| assert!(editor.read(cx).completion.is_none()));
}

#[gpui::test]
fn clicking_an_unquoted_option_uses_dialect_quotes_and_rejects_edits_to_markdown(
    cx: &mut TestAppContext,
) {
    let draft = "```mysql\r\nselect acct.items.or\r\n```\r\n\r\nOutside.";
    let head = draft.find(".or").unwrap() + 3;
    let schema = SqlNamespace::Namespaces(vec![(
        "acct.items".into(),
        SqlNamespace::Completions(vec!["order number".into()]),
    )]);
    for reject in [false, true] {
        let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(draft.into(), window, cx));
        let executor = cx.update(|window, cx| {
            bind_keys(cx);
            editor.update(cx, |editor, cx| {
                activate_draft(editor, head, window, cx);
                configure(
                    editor,
                    Extension::group([
                        schema_extension(SqlDialect::MySql, &schema),
                        transaction_filter(move |_, tx| {
                            if reject && tx.annotation(picked_completion()).is_some() {
                                Ok(Transaction::new(
                                    ChangeSet::single(Change::new(0..3, "bad")).unwrap(),
                                ))
                            } else {
                                Ok(tx)
                            }
                        }),
                    ])
                    .append(),
                    cx,
                );
            });
            cx.background_executor().clone()
        });
        cx.simulate_keystrokes("ctrl-space");
        executor.advance_clock(Duration::from_millis(50));
        cx.run_until_parked();
        let bounds = cx.debug_bounds("source-completion: order number").unwrap();
        cx.simulate_click(bounds.center(), Modifiers::none());
        cx.update(|_, cx| {
            let editor = editor.read(cx);
            if reject {
                assert_eq!(editor.source(), draft);
                assert_eq!(editor.cursor(), head);
                assert!(!editor.view.can_undo());
            } else {
                assert_eq!(editor.source(), draft.replace(".or", ".`order number`"));
                assert_eq!(editor.cursor(), head - 2 + "`order number`".len());
                assert_eq!(editor.view.undo_depth(), 1);
            }
        });
        if !reject {
            cx.simulate_keystrokes("cmd-z");
            cx.update(|_, cx| assert_eq!(editor.read(cx).source(), draft));
        }
    }
}
