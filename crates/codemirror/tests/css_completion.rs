use gpui_editor_codemirror::{
    Change, ChangeSet, EditorSelection, EditorState, Text, Transaction,
    autocomplete::session::{
        CompletionConfig, CompletionSession, CompletionStatus, SourceId, SourceResult,
    },
    language::css::{
        CssDocument, CssNodeKind, css_completion_source, define_css_completion_source,
    },
};
use std::time::Duration;

// Captured with `css()`, `EditorState.languageDataAt`, and the real
// `CompletionContext` from @codemirror/autocomplete 6.20.4. See the pins in
// docs/licenses/codemirror-lang-css.txt. Properties were captured in Chromium
// 159.0.8154.6; the other options do not depend on a browser.
// Each answer contains the replacement start in bytes and an ordered option set.
#[derive(Clone, Copy)]
enum Options {
    Properties,
    Values,
    Pseudos,
    Tags,
    AtRules,
    Empty,
    OneVariable,
    ThreeVariables,
    EscapedVariable,
    CurrentVariable,
    LargeSheetVariables,
}

impl Options {
    fn signature(self) -> (usize, u32) {
        match self {
            Self::Properties => (741, 283_478_746),
            Self::Values => (561, 3_770_060_376),
            Self::Pseudos => (63, 335_538_835),
            Self::Tags => (78, 1_288_706_251),
            Self::AtRules => (19, 3_639_269_078),
            Self::Empty => (0, 2_166_136_261),
            Self::OneVariable => (1, 3_610_230_257),
            Self::ThreeVariables => (3, 175_767_205),
            Self::EscapedVariable => (2, 2_778_394_448),
            Self::CurrentVariable => (1, 456_398_394),
            Self::LargeSheetVariables => (2, 4_177_022_331),
        }
    }
}

fn assert_oracle(result: Option<SourceResult>, expected: Option<(usize, Options)>, name: &str) {
    match (result, expected) {
        (None, None) => {}
        (Some(result), Some((from, options))) => {
            assert_eq!((result.from, result.to), (from, None), "{name}");
            assert!(result.filter, "{name}");
            assert!(result.commit_characters.is_empty(), "{name}");
            // FNV-1a over every label, type, and apply string, in upstream order.
            let mut hash = 2_166_136_261u32;
            for item in result.options.iter() {
                assert!(
                    item.display_label.is_none() && item.sort_text.is_none(),
                    "{name}"
                );
                assert!(item.detail.is_none() && item.info.is_none(), "{name}");
                assert!(
                    item.section.is_none() && item.commit_characters.is_none(),
                    "{name}"
                );
                assert_eq!(item.boost, 0, "{name}");
                for byte in item
                    .label
                    .bytes()
                    .chain([0])
                    .chain(item.kind.as_deref().unwrap().bytes())
                    .chain([0])
                    .chain(item.insert.as_deref().unwrap_or_default().bytes())
                    .chain([b'\n'])
                {
                    hash = (hash ^ u32::from(byte)).wrapping_mul(16_777_619);
                }
            }
            assert_eq!((result.options.len(), hash), options.signature(), "{name}");
        }
        (actual, expected) => panic!(
            "{name}: result {}, expected {}",
            actual.is_some(),
            expected.is_some()
        ),
    }
}

fn document_at(input: &str) -> (CssDocument, usize) {
    let position = input.find('§').unwrap();
    (
        CssDocument::parse(Text::new(input.replace('§', ""))),
        position,
    )
}

#[test]
fn matches_pinned_css_completion_contexts_and_options() {
    use Options::*;

    let check = |input, implicit, explicit| {
        let (doc, position) = document_at(input);
        for (is_explicit, expected) in [(false, implicit), (true, explicit)] {
            assert_oracle(
                css_completion_source(&doc, position, is_explicit),
                expected,
                &format!("{input:?}, explicit={is_explicit}"),
            );
        }
    };
    for (input, from, options) in [
        ("a§", 0, Tags),
        ("di§v {}", 0, Tags),
        ("a { col§or: red }", 4, Properties),
        ("a { col§ }", 4, Properties),
        ("a { col§", 4, Properties),
        ("a { -§ }", 4, Properties),
        ("a { --§ }", 4, Empty),
        ("a { --§", 4, Empty),
        ("a { color: re§d }", 11, Values),
        ("a:ho§ver {}", 2, Pseudos),
        ("a::be§fore {}", 3, Pseudos),
        ("@me§", 0, AtRules),
        ("a { @me§ }", 4, AtRules),
        ("a { foo§ {} }", 4, Properties),
        ("a { a:ho§ver {} }", 6, Pseudos),
        ("div > sp§an {}", 6, Tags),
        ("a { color: rgb(1, blu§e) }", 18, Values),
        ("a { color: var(-§) }", 15, Empty),
        ("a { --one: 1; color: var(--§) }", 25, OneVariable),
        ("a { --one: 1; color: var(--§foo) }", 25, OneVariable),
        (
            "a { color: var(--§); --one: 1; --one: 2; --Two: 3; } b { --two: 4; z: var(--ignored); --stray }",
            15,
            ThreeVariables,
        ),
        ("a { --one: 1; color: var(--missing, -§) }", 36, OneVariable),
        ("a { --one: 1; color: var(--thing§", 25, OneVariable),
        ("a { --one: 1; color: calc(--§) }", 26, OneVariable),
        ("a { --one: 1; color: --§ }", 21, OneVariable),
        ("a { --one: 1; --§ }", 14, OneVariable),
        ("--top§", 0, Tags),
        (".é { /*😀*/ \r\n col§or: red }", 18, Properties),
        ("@supports (display: gri§d) {}", 20, Values),
        (
            "a { color: ordinary§; } b { width: 1px; display: flex; }",
            11,
            Values,
        ),
        ("a { color: var( -§ /*foo*/ ) }", 16, Empty),
        ("a { --fo§ }", 4, Empty),
        ("a { --fo§; }", 4, Empty),
        ("a { --fo§ {} }", 4, Empty),
        ("a { --fo§ .b }", 4, Empty),
        ("a { --fo§ .b {} }", 4, Empty),
        ("a { --fo§/*x*/ }", 4, Empty),
        ("a { --fo§: red }", 4, CurrentVariable),
    ] {
        check(input, Some((from, options)), Some((from, options)));
    }
    for (input, from, options) in [
        ("a { § }", 4, Properties),
        ("a {§}", 3, Properties),
        ("a { color:§ }", 10, Properties),
        ("a { color: § }", 11, Properties),
        ("a { color: red; § }", 16, Properties),
        ("a { color: red § }", 15, Properties),
        ("a:§ {}", 2, Pseudos),
        ("a: § {}", 3, Pseudos),
        ("a { color: rgb(§) }", 15, Values),
        ("a { color: rgb(1, §) }", 18, Values),
        ("a { color: var(§) }", 15, Empty),
        ("a { color: var(§", 15, Empty),
        ("a { color: VAR(§) }", 15, Values),
        ("a { color: var( § ) }", 16, Empty),
        ("a { color: var(red, §) }", 20, Empty),
        (
            "a { --one/**/: 1; --two: 2; --escaped\\31: 3; color: var(§) }",
            56,
            EscapedVariable,
        ),
        ("a { --one: 1; color: var(--missing, §) }", 36, OneVariable),
        ("a { color:§ red }", 10, Values),
        ("a { color: var(-§, red) }", 16, Values),
        ("a { color: var(red -§) }", 20, Values),
        ("a { @§ }", 5, Properties),
        ("a { @§; }", 5, Properties),
        ("a:§ b {}", 2, Pseudos),
        ("a > b:§ {}", 6, Pseudos),
    ] {
        check(input, None, Some((from, options)));
    }
    for input in [
        "§",
        "§div {}",
        "a {§",
        "a { §",
        "a { -§",
        "a { color: §",
        "a { color:§",
        "a { color: red;§",
        "a { width: 1p§x }",
        "a:§",
        "a::§",
        "a::§ {}",
        "@med§ia (width > 1px) {}",
        "@§",
        "@media§",
        "@media §",
        ".cl§ass {}",
        "#i§d {}",
        "a { color: \"re§d\"; /* foo */ }",
        "a { /* foo§ */ }",
        "a { color: /* foo§ */ red }",
        "a { color: rgb(§",
        "a { color: var(-§ }",
        "a { --one: 1; color: var(-§",
        "@media (width: §) {}",
        "a { width: calc(3px + §1px) }",
        "a { width: calc§(3px) }",
        "a { color: var(-§, red }",
        "a { @§",
        "@§ {}",
        "@§; a {}",
        "a:§{}",
        "a:hover § {}",
    ] {
        check(input, None, None);
    }
}

#[test]
fn variable_names_preserve_document_order_across_large_stylesheets() {
    let source =
        "a { --one: 1; color: var(--§); }".to_owned() + &"\nb { --two: 2; --one: 3; }".repeat(200);
    let (doc, position) = document_at(&source);
    assert_oracle(
        css_completion_source(&doc, position, false),
        Some((25, Options::LargeSheetVariables)),
        "variables across large stylesheets",
    );
}

#[test]
fn css_completion_works_with_session_filtering_acceptance_and_valid_for() {
    let (mut state, mut session, position) = start_session("a { col§ }");
    let id = session
        .options()
        .unwrap()
        .iter()
        .find(|option| option.item.label == "color")
        .unwrap()
        .id;
    let transaction = session
        .accept(id, Duration::from_millis(200))
        .unwrap()
        .unwrap();
    let mut accepted = state.clone();
    accepted.update(transaction).unwrap();
    assert_eq!(accepted.text().as_str(), "a { color:  }");
    type_at(&mut state, &mut session, position, "o", 220);
    assert_eq!(session.status(), CompletionStatus::Active);
    assert!(
        session
            .options()
            .unwrap()
            .iter()
            .any(|option| option.id == id)
    );
    type_at(&mut state, &mut session, position + 1, "é", 221);
    assert_eq!(session.status(), CompletionStatus::Pending);

    let (mut state, mut session, position) = start_session("a { --tone: red; color: var(-§) }");
    type_at(&mut state, &mut session, position, "-t", 220);
    assert_eq!(session.status(), CompletionStatus::Active);
    assert_eq!(session.options().unwrap()[0].item.label, "--tone");
    type_at(&mut state, &mut session, position + 2, "é", 221);
    assert_eq!(session.status(), CompletionStatus::Pending);

    // The upstream directive source uses the identifier regex. It does not
    // accept a leading @, so the session must request a fresh result on typing.
    let (mut state, mut session, position) = start_session("@me§");
    type_at(&mut state, &mut session, position, "d", 220);
    assert_eq!(session.status(), CompletionStatus::Pending);
}

fn start_session(input: &str) -> (EditorState, CompletionSession, usize) {
    let (doc, position) = document_at(input);
    let mut state = EditorState::new(doc.text().as_str());
    state
        .update(
            Transaction::new(ChangeSet::default())
                .with_selection(EditorSelection::single(position, position)),
        )
        .unwrap();
    let mut session = CompletionSession::new(&state, CompletionConfig::default(), [SourceId(1)]);
    session.start(true, Duration::ZERO);
    let query_time = session.deadline().unwrap();
    let [query] = session.take_queries(query_time).try_into().unwrap();
    let doc = CssDocument::parse(query.state().text().clone());
    assert!(
        session
            .finish_query(
                &query,
                css_completion_source(&doc, query.position, query.explicit),
                query_time
            )
            .unwrap()
    );
    assert_eq!(session.status(), CompletionStatus::Active);
    (state, session, position)
}

fn type_at(
    state: &mut EditorState,
    session: &mut CompletionSession,
    position: usize,
    text: &str,
    time: u64,
) {
    let changes = ChangeSet::single(Change::new(position..position, text)).unwrap();
    let desc = changes.desc(state.text().len()).unwrap();
    let end = position + text.len();
    state
        .update(
            Transaction::new(changes)
                .with_selection(EditorSelection::single(end, end))
                .with_user_event("input.type"),
        )
        .unwrap();
    session
        .update(
            state,
            &desc,
            true,
            Some("input.type"),
            Duration::from_millis(time),
        )
        .unwrap();
}

#[test]
fn css_completion_rejects_invalid_byte_positions() {
    let doc = CssDocument::parse(Text::new(".é { color: red }"));
    assert!(css_completion_source(&doc, 2, true).is_none());
    assert!(css_completion_source(&doc, doc.text().len() + 1, true).is_none());
}

#[test]
fn dialect_source_uses_its_variable_predicate_with_upstream_precedence() {
    let source = define_css_completion_source(|node| {
        matches!(
            node.kind(),
            CssNodeKind::PropertyName | CssNodeKind::TagName
        )
    });
    for (input, explicit) in [
        ("a§ { color: var(); } b { width: 1px; }", false),
        ("a { color: var(§); } b { width: 1px; }", true),
    ] {
        let (doc, position) = document_at(input);
        let result = source(&doc, position, explicit).unwrap();
        let names: Vec<_> = result
            .options
            .iter()
            .map(|item| (item.label.as_str(), item.kind.as_deref()))
            .collect();
        assert_eq!(
            names,
            [("color", Some("variable")), ("width", Some("variable"))]
        );
        assert_eq!(result.from, if explicit { position } else { 0 });
    }
    let (doc, position) = document_at("a { col§or: red }");
    assert_oracle(
        source(&doc, position, false),
        Some((4, Options::Properties)),
        "PropertyName takes precedence",
    );
}
