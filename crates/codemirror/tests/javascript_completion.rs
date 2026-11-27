//! Fixtures from the SRI-verified `@codemirror/lang-javascript` 6.2.5 tarball.
//! The scratch oracle used `@codemirror/autocomplete` 6.20.4, `@codemirror/state`
//! 6.7.6, `@codemirror/language` 6.13.1, `@lezer/javascript` 1.5.7,
//! `@lezer/lr` 1.4.13, and `@lezer/common` 1.5.7. Positions use UTF-8 bytes.

use gpui_editor_codemirror::{
    EditorSelection, EditorState, Text,
    autocomplete::session::SourceResult,
    language::javascript::{
        JavascriptDocument, JavascriptOptions, JavascriptScope, JavascriptScopeProperty,
        JavascriptScopeValue, SNIPPETS, TYPESCRIPT_SNIPPETS, completion_path,
        javascript_completion_snippet, javascript_completion_source, local_completion_source,
        scope_completion_source,
    },
};
use serde_json::{Value, json};

fn marked(source: &str, options: JavascriptOptions) -> (JavascriptDocument, usize) {
    let position = source.find('¦').unwrap();
    let source = source.replace('¦', "");
    (
        JavascriptDocument::parse_with_options(Text::new(source), options),
        position,
    )
}

fn labels(result: &SourceResult) -> Vec<String> {
    result
        .options
        .iter()
        .map(|item| format!("{}:{}", item.kind.as_deref().unwrap(), item.label))
        .collect()
}

#[test]
fn syntax_completion_contexts_match_the_pinned_sources() {
    let cases: Vec<Value> =
        serde_json::from_str(include_str!("fixtures/javascript_completion_contexts.json")).unwrap();
    for case in cases {
        let id = case["id"].as_str().unwrap();
        let options = JavascriptOptions {
            typescript: case["typescript"] == true,
            jsx: case["jsx"] == true,
        };
        let explicit = case["explicit"] == true;
        let (doc, position) = marked(case["markup"].as_str().unwrap(), options);
        let local = local_completion_source(&doc, position, explicit);
        let path = completion_path(&doc, position, explicit);
        let fixed = javascript_completion_source(&doc, position, explicit, options.typescript);
        assert_eq!(
            json!(local.as_ref().map(|value| (value.from, labels(value)))),
            case["local"],
            "local: {id}"
        );
        assert_eq!(
            json!(path.map(|value| json!({"path":value.path,"name":value.name}))),
            case["path"],
            "path: {id}"
        );
        assert_eq!(
            json!(fixed.as_ref().map(|value| value.from)),
            case["fixed"],
            "fixed: {id}"
        );
        for result in local.iter().chain(fixed.iter()) {
            assert!(
                result.filter && result.to.is_none() && result.commit_characters.is_empty(),
                "result metadata: {id}"
            );
        }
    }
}

#[test]
fn the_fixed_list_has_the_pinned_entries_in_order() {
    let js_keywords = "break case const continue default delete export extends false finally in \
        instanceof let new return static super switch this throw true typeof var yield";
    let ts_keywords = "declare implements private protected public";
    let snippet_labels = [
        "function:definition",
        "for:loop",
        "for:of loop",
        "do:loop",
        "while:loop",
        "try:/ catch block",
        "if:block",
        "if:/ else block",
        "class:definition",
        "import:named",
        "import:default",
        "interface:definition",
        "type:definition",
        "enum:definition",
    ];
    for (typescript, count, snippets, extra_keywords) in [
        (false, 11, SNIPPETS, ""),
        (true, 14, TYPESCRIPT_SNIPPETS, ts_keywords),
    ] {
        let doc = JavascriptDocument::parse(Text::new(""));
        let fixed = javascript_completion_source(&doc, 0, true, typescript).unwrap();
        let mut expected = snippet_labels[..count].to_vec();
        expected.extend(
            js_keywords
                .split_whitespace()
                .chain(extra_keywords.split_whitespace()),
        );
        let actual: Vec<_> = fixed
            .options
            .iter()
            .map(|item| {
                assert_eq!(item.kind.as_deref(), Some("keyword"));
                assert!(item.insert.is_none());
                item.detail.as_ref().map_or_else(
                    || item.label.clone(),
                    |detail| format!("{}:{detail}", item.label),
                )
            })
            .collect();
        assert_eq!(actual, expected);
        assert_eq!(snippets.len(), count);
        for (option, expected) in fixed.options.iter().zip(snippets) {
            assert_eq!(javascript_completion_snippet(option), Some(expected));
        }
        for option in &fixed.options[count..] {
            assert!(javascript_completion_snippet(option).is_none());
        }
    }
}

#[test]
fn every_catalog_snippet_matches_upstream_text_and_field_stops() {
    let cases: Vec<Value> =
        serde_json::from_str(include_str!("fixtures/javascript_completion_snippets.json")).unwrap();
    assert_eq!(cases.len(), TYPESCRIPT_SNIPPETS.len());
    let ranges = |selection: &EditorSelection| {
        selection
            .ranges()
            .iter()
            .map(|site| [site.range().start, site.range().end])
            .collect::<Vec<_>>()
    };
    for (snippet, case) in TYPESCRIPT_SNIPPETS.iter().zip(cases) {
        assert_eq!(snippet.label, case["label"]);
        assert_eq!(snippet.detail, case["detail"]);
        let mut state = EditorState::new("  xx");
        let (insert, session) = snippet.snippet().insert(&state, 2..4, "  ").unwrap();
        state.update(insert).unwrap();
        assert_eq!(state.text().as_str(), case["text"]);
        let mut actual = vec![ranges(state.selection())];
        if let Some(mut session) = session {
            while let Some(selection) = session.next_field() {
                actual.push(ranges(&selection));
            }
        }
        assert_eq!(
            json!(actual),
            case["stops"],
            "{}:{}",
            snippet.label,
            snippet.detail
        );
    }
}

#[test]
fn scope_objects_match_prototype_order_shadowing_and_option_kinds() {
    use JavascriptScopeValue::{Function, Object, Unavailable, Value};

    let property = |name, value| JavascriptScopeProperty { name, value };
    let nested = JavascriptScope {
        properties: &[property("deep", Value)],
        prototypes: &[],
    };
    let object = JavascriptScope {
        properties: &[
            property("field", Value),
            property("method", Function(None)),
            property("Éclair", Value),
            property("bad-key", Value),
            property("😀", Value),
        ],
        prototypes: &[&[
            property("field", Value),
            property("Render", Function(None)),
            property("fallback", Value),
            property("nested", Object(&nested)),
        ]],
    };
    let root = JavascriptScope {
        properties: &[
            property("obj", Object(&object)),
            property("App", Function(None)),
            property("lower", Function(None)),
            property("count", Value),
            property("missing", Value),
            property("𐐀", Object(&object)),
            property("fault", Unavailable),
        ],
        prototypes: &[&[
            property("rootFn", Function(None)),
            property("inherited", Value),
            property("through", Object(&object)),
            property("missing", Object(&object)),
        ]],
    };
    let source = scope_completion_source(&root);
    let root_options = "obj:variable:0 App:class:0 lower:function:0 count:variable:0 missing:variable:0 \
        𐐀:variable:0 rootFn:function:-1 inherited:variable:-1 through:variable:-1";
    let object_options = "field:property:0 method:method:0 Éclair:property:0 😀:property:0 \
        Render:class:-1 fallback:property:-1 nested:property:-1";
    for (input, explicit, from, options) in [
        ("¦", true, 0, root_options),
        ("obj.¦", false, 4, object_options),
        ("obj.fi¦eld", false, 1, object_options),
        ("obj.nested.¦", false, 11, "deep:property:0"),
        ("through.¦", false, 8, object_options),
        ("obj.Éc¦lair", false, 0, object_options),
        ("𐐀.foo¦bar", false, 0, object_options),
    ] {
        let (doc, position) = marked(input, JavascriptOptions::default());
        let result = source(&doc, position, explicit).unwrap_or_else(|| panic!("{input}"));
        assert_eq!(result.from, from, "{input}");
        assert_eq!(result.to, None, "{input}");
        assert!(result.filter, "{input}");
        let actual: Vec<_> = result
            .options
            .iter()
            .map(|option| {
                format!(
                    "{}:{}:{}",
                    option.label,
                    option.kind.as_deref().unwrap(),
                    option.boost
                )
            })
            .collect();
        assert_eq!(
            actual,
            options.split_whitespace().collect::<Vec<_>>(),
            "{input}"
        );
    }
    for input in [
        "missing.¦",
        "unknown.¦",
        "factory().f¦",
        "obj[\"field\"].f¦",
        "𐐀.foo¦ba",
    ] {
        let (doc, position) = marked(input, JavascriptOptions::default());
        assert!(source(&doc, position, false).is_none(), "{input}");
    }
}

#[test]
fn sources_reject_invalid_byte_positions_and_bound_keyword_matches() {
    let doc = JavascriptDocument::parse(Text::new("é"));
    let scope = JavascriptScope::default();
    for position in [1, 3, usize::MAX] {
        assert!(javascript_completion_source(&doc, position, true, false).is_none());
        assert!(local_completion_source(&doc, position, true).is_none());
        assert!(completion_path(&doc, position, true).is_none());
        assert!(scope_completion_source(&scope)(&doc, position, true).is_none());
    }
    let doc = JavascriptDocument::parse(Text::new(format!("𐐀{}", "a".repeat(300))));
    let end = doc.text().len();
    assert_eq!(
        javascript_completion_source(&doc, end, false, false)
            .unwrap()
            .from,
        end - 250
    );
    assert_eq!(local_completion_source(&doc, end, false).unwrap().from, 0);
}
