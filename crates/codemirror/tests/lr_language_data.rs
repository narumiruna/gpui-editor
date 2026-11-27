use std::{ops::Range, sync::Arc};

use gpui_editor_codemirror::{
    Assoc, Compartment, EditorState, Extension, Precedence, Transaction,
    commands::CommentTokens,
    language::{
        LanguageData, language_data_provider,
        markdown::{
            MarkdownCodeLanguages, MarkdownConfig, MarkdownParserConfig, markdown_snapshot,
            markdown_with_parser,
        },
    },
    language_data::{
        CloseBracketData, JsonLanguage, LANGUAGES, LanguageDescription, LezerLanguage, LrDataFacet,
        NativeLanguage, attached_language_snapshot, json_extension, json_language, lezer_extension,
        lezer_language,
    },
};
use serde_json::{Value, json};

#[derive(Clone)]
enum Handle {
    Json(JsonLanguage),
    Lezer(LezerLanguage),
}

impl Handle {
    fn stock(package: &str) -> Self {
        match package {
            "json" => Self::Json(json_language().clone()),
            "lezer" => Self::Lezer(lezer_language().clone()),
            _ => unreachable!(),
        }
    }

    fn configure(&self, strict: Option<bool>, name: Option<&str>) -> Self {
        match self {
            Self::Json(handle) => Self::Json(handle.configure(strict, name)),
            Self::Lezer(handle) => Self::Lezer(handle.configure(strict, name)),
        }
    }

    fn facet(&self) -> &'static LrDataFacet {
        match self {
            Self::Json(handle) => handle.data_facet(),
            Self::Lezer(handle) => handle.data_facet(),
        }
    }

    fn data(&self, data: LanguageData) -> Extension {
        match self {
            Self::Json(handle) => handle.data(data),
            Self::Lezer(handle) => handle.data(data),
        }
    }

    fn extension(&self) -> Extension {
        match self {
            Self::Json(handle) => handle.extension(),
            Self::Lezer(handle) => handle.extension(),
        }
    }

    fn factory(&self) -> Extension {
        match self {
            Self::Json(_) => json_extension(),
            Self::Lezer(_) => lezer_extension(),
        }
    }

    fn grammar(&self) -> NativeLanguage {
        match self {
            Self::Json(_) => NativeLanguage::Json,
            Self::Lezer(_) => NativeLanguage::LezerGrammar,
        }
    }

    fn active(&self, state: &EditorState, at: usize) -> [bool; 2] {
        [Assoc::Before, Assoc::After].map(|side| match self {
            Self::Json(handle) => handle.is_active_at(state, at, side),
            Self::Lezer(handle) => handle.is_active_at(state, at, side),
        })
    }

    fn regions(&self, state: &EditorState) -> Vec<[usize; 2]> {
        let ranges: Vec<Range<usize>> = match self {
            Self::Json(handle) => handle.find_regions(state),
            Self::Lezer(handle) => handle.find_regions(state),
        };
        ranges
            .into_iter()
            .map(|range| [range.start, range.end])
            .collect()
    }
}

fn fixture(kind: &str) -> impl Iterator<Item = Value> {
    include_str!("fixtures/lr_language_data.jsonl")
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .filter(move |row| row["kind"] == kind)
}

fn words(state: &EditorState, at: usize, side: Assoc) -> Vec<&'static str> {
    state.language_data_at("wordChars", at, side)
}

fn root_data(word: &'static str) -> LanguageData {
    LanguageData::default().with("wordChars", word)
}

fn format_data(
    handle: &Handle,
    word: &'static str,
    brackets: &'static [&'static str],
    line: &'static str,
) -> Extension {
    let data = root_data(word);
    handle.data(match handle {
        Handle::Json(_) => data.with(
            "closeBrackets",
            CloseBracketData {
                brackets: Some(brackets),
                ..Default::default()
            },
        ),
        Handle::Lezer(_) => data.with(
            "commentTokens",
            CommentTokens {
                line: Some(line),
                block: None,
            },
        ),
    })
}

fn metadata(state: &EditorState, handle: &Handle) -> Value {
    match handle {
        Handle::Json(_) => json!(
            state
                .language_data_at::<CloseBracketData>("closeBrackets", 0, Assoc::After)
                .into_iter()
                .map(|datum| datum.brackets)
                .collect::<Vec<_>>()
        ),
        Handle::Lezer(_) => json!(
            state
                .language_data_at::<CommentTokens>("commentTokens", 0, Assoc::After)
                .into_iter()
                .map(|datum| (datum.line, datum.block))
                .collect::<Vec<_>>()
        ),
    }
}

#[test]
fn pinned_stock_metadata_uses_facet_precedence_and_active_provider_identity() {
    for row in fixture("install") {
        let package = row["package"].as_str().unwrap();
        let source = row["source"].as_str().unwrap();
        let stock = Handle::stock(package);
        let configured = stock.configure(Some(true), Some("renamed"));
        let another = configured.configure(None, Some(""));
        let other = Handle::stock(if package == "json" { "lezer" } else { "json" });
        assert!(stock.facet() == configured.facet());
        assert!(another.facet() == configured.facet());
        assert!(other.facet() != stock.facet());
        let high = format_data(&configured, "@", &["("], "#").with_precedence(Precedence::High);
        let ordinary = format_data(&stock, "&", &["'"], "--");
        let low = format_data(&stock, "$", &["<"], ";").with_precedence(Precedence::Low);
        let wrong = other
            .data(root_data("wrong"))
            .with_precedence(Precedence::Highest);
        let step = row["step"].as_str().unwrap();
        let install = match step {
            "only entries" => vec![],
            "stock" => vec![stock.extension()],
            "stock factories" => vec![stock.extension(), stock.factory(), stock.factory()],
            "same parser twice" => vec![
                configured.extension(),
                configured.extension().with_precedence(Precedence::High),
            ],
            "distinct same name" => vec![configured.extension(), another.extension()],
            "stock and renamed" => vec![stock.extension(), configured.extension()],
            "renamed wins" => vec![
                stock.extension(),
                configured.extension().with_precedence(Precedence::High),
            ],
            "other loses" => vec![stock.extension(), other.extension()],
            "other wins" => vec![
                stock.extension(),
                other.extension().with_precedence(Precedence::High),
            ],
            _ => unreachable!(),
        };
        let active_at = |handle: &Handle, state: &EditorState| {
            [handle.active(state, 0), handle.active(state, source.len())]
        };
        let extensions = Extension::group(
            std::iter::once(low)
                .chain(install)
                .chain([ordinary, high, wrong]),
        );
        let state = EditorState::with_extensions(source, extensions).unwrap();
        let winner =
            JsonLanguage::active_name(&state).or_else(|| LezerLanguage::active_name(&state));
        assert_eq!(json!(winner.as_deref()), row["winner"], "{package}: {step}");
        assert_eq!(
            json!(words(&state, 0, Assoc::Before)),
            row["words"],
            "{package}: {step}"
        );
        assert_eq!(metadata(&state, &stock), row["values"], "{package}: {step}");
        assert_eq!(
            json!([active_at(&stock, &state), active_at(&configured, &state)]),
            row["active"],
            "{package}: {step}"
        );
        assert_eq!(
            json!([stock.regions(&state), configured.regions(&state)]),
            row["regions"],
            "{package}: {step}"
        );
        let invalid = source.find('😀').unwrap() + 1;
        assert_eq!(stock.active(&state, invalid), [false, false]);
        assert!(words(&state, invalid, Assoc::Before).is_empty());
    }
}

#[test]
fn pinned_data_leaf_dedup_and_empty_documents() {
    for row in fixture("leaves") {
        let stock = Handle::stock(row["package"].as_str().unwrap());
        let leaf = stock.facet().of(root_data("one"));
        let host = language_data_provider(|_, _, _| vec![Arc::new(root_data("host"))])
            .with_precedence(Precedence::Highest);
        let state = EditorState::with_extensions(
            "",
            Extension::group([
                host,
                stock.extension(),
                leaf.clone(),
                leaf.with_precedence(Precedence::High),
                stock.data(root_data("one")),
            ]),
        )
        .unwrap();
        assert_eq!(json!(words(&state, 0, Assoc::After)), row["words"]);
    }
    for row in fixture("empty") {
        let stock = Handle::stock(row["package"].as_str().unwrap());
        let state = EditorState::with_extensions("", stock.extension()).unwrap();
        assert_eq!(
            json!(stock.configure(Some(true), None).active(&state, 0)),
            row["active"]
        );
        assert_eq!(json!(stock.regions(&state)), row["regions"]);
    }
}

#[test]
fn pinned_data_reconfiguration_keeps_the_parser_snapshot_and_retained_values() {
    for package in ["json", "lezer"] {
        let mut rows = fixture("swap")
            .filter(|row| row["package"] == package)
            .peekable();
        let source = rows.peek().unwrap()["source"].as_str().unwrap().to_owned();
        let stock = Handle::stock(package);
        let configured = stock.configure(Some(true), Some("renamed"));
        let another = configured.configure(None, Some(""));
        let parser = Compartment::new();
        let data = Compartment::new();
        let retained = EditorState::with_extensions(
            &source,
            Extension::group([
                parser.of(stock.factory()),
                data.of(Extension::group([
                    stock.data(root_data("$")).with_precedence(Precedence::Low),
                    configured
                        .data(root_data("@"))
                        .with_precedence(Precedence::High),
                ])),
            ]),
        )
        .unwrap();
        let mut state = retained.clone();
        for row in rows {
            let step = row["step"].as_str().unwrap();
            let before = attached_language_snapshot(&state).unwrap();
            let effect = match step {
                "factory again" => parser.reconfigure(stock.factory()),
                "new parser" => parser.reconfigure(configured.extension()),
                "distinct same name" => parser.reconfigure(another.extension()),
                "remove data" => data.reconfigure(Extension::default()),
                "detach" => parser.reconfigure(Extension::default()),
                _ => unreachable!(),
            };
            state
                .update(Transaction::default().with_effects([effect]))
                .unwrap();
            let reused = attached_language_snapshot(&state)
                .is_some_and(|current| Arc::ptr_eq(&before, &current));
            assert_eq!(json!(reused), row["reused"], "{package}: {step}");
            assert_eq!(json!(words(&state, 0, Assoc::Before)), row["words"]);
            assert_eq!(json!(words(&retained, 0, Assoc::Before)), row["retained"]);
            assert_eq!(json!(stock.regions(&state)), row["regions"]);
        }
    }
}

#[test]
fn pinned_direct_markdown_mounts_expose_stock_data_at_both_fence_endpoints() {
    for row in fixture("mount") {
        let stock = Handle::stock(row["package"].as_str().unwrap());
        let configured = stock.configure(Some(true), None);
        let grammar = stock.grammar();
        let mode = row["mode"].as_str().unwrap();
        let code_languages = match mode {
            "plain" | "indented" => None,
            "resolver" | "empty" | "unclosed" => {
                Some(MarkdownCodeLanguages::Resolve(Arc::new(move |_| {
                    Some(grammar)
                })))
            }
            "catalog" => Some(MarkdownCodeLanguages::Catalog(LANGUAGES.into())),
            _ => unreachable!(),
        };
        let source = row["source"].as_str().unwrap();
        let extension = markdown_with_parser(
            MarkdownConfig::default(),
            MarkdownParserConfig {
                code_languages,
                default_code_language: (mode == "indented").then_some(grammar),
                ..Default::default()
            },
        );
        let state = EditorState::with_extensions(
            source,
            Extension::group([
                extension,
                stock.data(root_data("$")).with_precedence(Precedence::Low),
                configured
                    .data(root_data("@"))
                    .with_precedence(Precedence::High),
            ]),
        )
        .unwrap();
        assert!(attached_language_snapshot(&state).is_none());
        let regions = configured.regions(&state);
        assert_eq!(
            json!(regions),
            row["regions"],
            "{} {}",
            row["package"],
            row["mode"]
        );
        for probe in row["probes"].as_array().unwrap() {
            let at = usize::try_from(probe[0].as_u64().unwrap()).unwrap();
            let actual = json!([
                at,
                configured.active(&state, at),
                words(&state, at, Assoc::Before),
                words(&state, at, Assoc::After)
            ]);
            assert_eq!(&actual, probe, "{} {} at {at}", row["package"], row["mode"]);
        }
        let mid = source.rfind("\r\n").unwrap() + 1;
        assert_eq!(configured.active(&state, mid), [false, false]);
        if let Some(astral) = source.find('😀') {
            assert_eq!(configured.active(&state, astral + 1), [false, false]);
        }
        if let Some([start, _]) = regions.first().copied() {
            let snapshot = markdown_snapshot(&state).unwrap();
            assert_ne!(snapshot.language_at(start, Assoc::Before), Some(grammar));
            assert!(
                snapshot
                    .data_at(start, Assoc::After)
                    .iter()
                    .all(|data| data.get::<&str>("wordChars").is_none())
            );
        }
    }
}

#[test]
fn catalog_json_uses_the_stock_attachment_and_no_lezer_entry_is_added() {
    let row = fixture("catalog").next().unwrap();
    assert_eq!(json!(LANGUAGES.len()), row["count"]);
    let entry = LanguageDescription::match_name(LANGUAGES, "json5", true).unwrap();
    assert_eq!(json!(entry.name), row["name"]);
    assert_eq!(json!(entry.aliases), row["aliases"]);
    assert_eq!(json!(entry.extensions), row["extensions"]);
    let load = entry.native_support_load();
    let support = load.load().unwrap();
    let handle = support.json_language().unwrap();
    assert!(handle == json_language());
    let state = EditorState::with_extensions(
        "{}",
        Extension::group([
            handle.extension(),
            json_language().data(root_data("loaded")),
        ]),
    )
    .unwrap();
    assert_eq!(words(&state, 0, Assoc::After), ["loaded"]);
    assert_eq!(JsonLanguage::active_name(&state).as_deref(), Some("json"));
    assert!(LanguageDescription::match_name(LANGUAGES, "lezer", true).is_none());
    assert!(LanguageDescription::match_filename(LANGUAGES, "x.lezer").is_none());
}
