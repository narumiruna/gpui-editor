//! Data from the SRI-verified `@codemirror/lang-sql` 6.10.0 tarball. The
//! scratch Node oracle used `@codemirror/autocomplete` 6.20.4, `@codemirror/state`
//! 6.7.6, `@codemirror/language` 6.13.1, `@lezer/lr` 1.4.13, and
//! `@lezer/common` 1.5.7. It converted UTF-16 offsets to UTF-8 byte positions.

use std::time::Duration;

use gpui_editor_codemirror::{
    Change, ChangeSet, EditorSelection, EditorState, Text, Transaction,
    autocomplete::{
        SectionRank,
        session::{
            CompletionConfig, CompletionItem, CompletionSession, CompletionStatus, SourceId,
            SourceResult,
        },
    },
    language::sql::{
        SqlDialect, SqlDialectSpec, SqlDocument, SqlKeywordCompletionSource, SqlLanguage,
        SqlNamespace, SqlSchemaConfig, SqlSchemaOption, keyword_completion_source,
        schema_completion_source,
    },
};
use serde_json::{Value, json};

fn fixture() -> Value {
    serde_json::from_str(include_str!("fixtures/sql_completion.json")).unwrap()
}

fn cases(value: &Value) -> &[Value] {
    value.as_array().unwrap()
}

fn marked(case: &Value) -> (Text, usize) {
    let source = case["markup"].as_str().unwrap();
    let position = source.find('¦').unwrap();
    (Text::new(source.replacen('¦', "", 1)), position)
}

fn dialect<'a>(data: &'a Value, name: &Value) -> SqlDialectSpec<'a> {
    let name = name.as_str().unwrap();
    if let Some(dialect) = SqlDialect::from_export(name) {
        return dialect.spec();
    }
    let data = &data["specs"][name];
    assert!(data.is_object(), "unknown dialect {name}");
    SqlDialectSpec {
        keywords: data["keywords"].as_str().unwrap_or_default(),
        types: data["types"].as_str().unwrap_or_default(),
        builtin: data["builtin"].as_str().unwrap_or_default(),
        identifier_quotes: data["identifierQuotes"].as_str().unwrap_or("\""),
        special_var: data["specialVar"].as_str().unwrap_or("?"),
        case_insensitive_identifiers: data["caseInsensitiveIdentifiers"] == true,
        hash_comments: data["hashComments"] == true,
        slash_comments: data["slashComments"] == true,
        space_after_dashes: data["spaceAfterDashes"] == true,
        ..SqlDialectSpec::DEFAULT
    }
}

fn item(value: &Value) -> CompletionItem {
    let string = |key: &str| value[key].as_str().map(str::to_owned);
    let mut item = CompletionItem::new(value["label"].as_str().unwrap());
    item.display_label = string("displayLabel");
    item.sort_text = string("sortText");
    item.kind = string("type");
    item.detail = string("detail");
    item.info = string("info");
    item.insert = string("apply");
    item.section = value["section"]["name"].as_str().map(|name| {
        let rank = value["section"]["rank"]
            .as_i64()
            .unwrap()
            .try_into()
            .unwrap();
        (name.to_owned(), SectionRank::Fixed(rank))
    });
    item.boost = value["boost"]
        .as_i64()
        .unwrap_or_default()
        .try_into()
        .unwrap();
    item.commit_characters = value["commitCharacters"].as_array().map(|values| {
        values
            .iter()
            .map(|ch| ch.as_str().unwrap().chars().next().unwrap())
            .collect()
    });
    item
}

fn project(item: &CompletionItem) -> Value {
    let section = item.section.as_ref().map(|(name, rank)| {
        let &SectionRank::Fixed(rank) = rank else {
            panic!("unexpected rank")
        };
        (name, rank)
    });
    json!([
        item.label,
        item.kind,
        item.boost,
        item.insert,
        item.detail,
        item.display_label,
        item.sort_text,
        item.info,
        item.commit_characters,
        section
    ])
}

fn keyword(language: &SqlLanguage, case: &Value) -> SqlKeywordCompletionSource {
    let upper = case["upper"] == true;
    let Some(builder) = case["builder"].as_str() else {
        return keyword_completion_source(language, upper);
    };
    SqlKeywordCompletionSource::with_builder(language, upper, |label, kind| {
        let mut item = CompletionItem::new(label);
        item.kind = Some(kind.to_owned());
        match builder {
            "rich" => {
                item.label = format!("~{label}");
                item.kind = Some(format!("sql-{kind}"));
                item.boost = 7;
                item.insert = Some(format!("put:{label}"));
                item.detail = Some(format!("d:{label}"));
                item.display_label = Some(format!("D:{label}"));
                item.sort_text = Some(format!("s:{label}"));
                item.info = Some(format!("i:{label}"));
                item.commit_characters = Some(vec!['.', ';']);
            }
            "single" => {
                item.label = match label {
                    "false" => "!",
                    "true" => "é",
                    "unknown" => "",
                    "null" => "𐀀",
                    _ => "x",
                }
                .to_owned()
            }
            "punct" => {
                item.label = match label {
                    "false" => "-x",
                    "true" => "😀q",
                    "unknown" => "[©]",
                    "null" => "\\🦀",
                    _ => ".9",
                }
                .to_owned()
            }
            "only" => return CompletionItem::new("!"),
            _ => panic!("unknown builder {builder}"),
        }
        item
    })
}

fn check_result(result: Option<&SourceResult>, case: &Value) {
    if let Some(result) = result {
        assert!(
            result.filter && result.commit_characters.is_empty(),
            "{case}"
        );
    }
    assert_eq!(
        json!(result.map(|result| (result.from, result.to))),
        case["result"],
        "{case}"
    );
}

#[test]
fn all_dialect_keyword_options_match_the_pinned_program() {
    let data = fixture();
    for case in cases(&data["keywordOptions"]) {
        let language = SqlLanguage::define(dialect(&data, &case["dialect"]));
        let source = keyword(&language, case);
        let result = source
            .complete(&language.parse(Text::default()), 0, true)
            .unwrap();
        assert_eq!(json!(result.options.len()), case["count"], "{case}");
        // FNV-1a over the JSON array of ordered options keeps the catalog
        // fixtures small while checking every label, type, and option field.
        let options: Vec<_> = result.options.iter().map(project).collect();
        let hash = serde_json::to_vec(&options)
            .unwrap()
            .into_iter()
            .fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
                (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
            });
        assert_eq!(json!(format!("{hash:016x}")), case["hash"], "{case}");
    }
}

#[test]
fn keyword_query_contexts_match_the_pinned_source() {
    let data = fixture();
    for case in cases(&data["keywordContexts"]) {
        let language = SqlLanguage::define(dialect(&data, &case["dialect"]));
        let (text, position) = marked(case);
        let doc = language.parse(text);
        let result = keyword(&language, case).complete(&doc, position, case["explicit"] == true);
        check_result(result.as_ref(), case);
    }
}

fn schema(value: &Value) -> SqlNamespace {
    match value[0].as_str().unwrap() {
        "items" => SqlNamespace::Completions(
            cases(&value[1])
                .iter()
                .map(|value| {
                    if let Some(name) = value.as_str() {
                        SqlSchemaOption::from(name)
                    } else {
                        item(value).into()
                    }
                })
                .collect(),
        ),
        "scope" => SqlNamespace::Namespaces(
            cases(&value[1])
                .iter()
                .map(|value| (value[0].as_str().unwrap().to_owned(), schema(&value[1])))
                .collect(),
        ),
        "self" => SqlNamespace::WithSelf {
            completion: Box::new(item(&value[1])),
            children: Box::new(schema(&value[2])),
        },
        _ => panic!("unknown schema level {value}"),
    }
}

fn schema_for(data: &Value, config: &Value) -> Option<SqlNamespace> {
    config.get("schema").map(|value| {
        schema(if value == "common" {
            &data["common"]
        } else {
            value
        })
    })
}

fn schema_source(
    config: &Value,
    spec: SqlDialectSpec<'_>,
    schema: Option<&SqlNamespace>,
) -> gpui_editor_codemirror::language::sql::SqlSchemaCompletionSource {
    let extras = |key: &str| {
        config[key]
            .as_array()
            .into_iter()
            .flatten()
            .map(item)
            .collect::<Vec<_>>()
    };
    schema_completion_source(SqlSchemaConfig {
        dialect: spec,
        schema,
        default_schema: config["defaultSchema"].as_str(),
        default_table: config["defaultTable"].as_str(),
        tables: &extras("tables"),
        schemas: &extras("schemas"),
    })
}

#[test]
fn schema_options_and_ranges_match_the_pinned_source() {
    let data = fixture();
    for (name, config) in data["schemaConfigs"].as_object().unwrap() {
        let spec = dialect(&data, &config["dialect"]);
        let language = SqlLanguage::define(spec);
        let source = schema_source(config, spec, schema_for(&data, config).as_ref());
        for case in cases(&data["schemaContexts"])
            .iter()
            .filter(|case| case["config"] == name.as_str())
        {
            let (text, position) = marked(case);
            let doc = language.parse(text);
            let result = source.complete(&doc, position, case["explicit"] == true);
            check_result(result.as_ref(), case);
            if let Some(result) = result {
                let index = usize::try_from(case["list"].as_u64().unwrap()).unwrap();
                let expected = &data["lists"][index];
                assert_eq!(
                    json!(result.options.iter().map(project).collect::<Vec<_>>()),
                    *expected,
                    "{case}"
                );
            }
        }
    }
}

#[test]
fn typing_reuses_source_results_only_for_valid_spans() {
    let data = fixture();
    for case in cases(&data["validity"]) {
        let (text, position) = marked(case);
        let mut state = EditorState::new(text.as_str());
        state
            .update(
                Transaction::new(ChangeSet::default())
                    .with_selection(EditorSelection::single(position, position)),
            )
            .unwrap();
        let result = if let Some(name) = case["config"].as_str() {
            let config = &data["schemaConfigs"][name];
            let spec = dialect(&data, &config["dialect"]);
            let doc = SqlLanguage::define(spec).parse(text);
            schema_source(config, spec, schema_for(&data, config).as_ref())
                .complete(&doc, position, true)
        } else {
            let language = SqlLanguage::define(dialect(&data, &case["dialect"]));
            let doc = language.parse(text);
            keyword(&language, case).complete(&doc, position, true)
        }
        .unwrap();

        let mut session =
            CompletionSession::new(&state, CompletionConfig::default(), [SourceId(1)]);
        session.start(true, Duration::ZERO);
        let [query] = session
            .take_queries(Duration::from_millis(50))
            .try_into()
            .unwrap();
        assert!(
            session
                .finish_query(&query, Some(result), Duration::from_millis(51))
                .unwrap()
        );
        let insert = case["insert"].as_str().unwrap();
        let changes = ChangeSet::single(Change::new(position..position, insert)).unwrap();
        let desc = changes.desc(state.text().len()).unwrap();
        let head = position + insert.len();
        state
            .update(Transaction::new(changes).with_selection(EditorSelection::single(head, head)))
            .unwrap();
        session
            .update(
                &state,
                &desc,
                true,
                Some("input.type"),
                Duration::from_millis(52),
            )
            .unwrap();
        assert_eq!(
            session.status() != CompletionStatus::Pending,
            case["valid"] == true,
            "{case}"
        );
    }
}

#[test]
fn ranges_reject_invalid_utf8_positions() {
    let doc = SqlDocument::parse(Text::new("select 😀"));
    let options = SqlNamespace::Completions(vec!["table".into()]);
    let schema = schema_completion_source(SqlSchemaConfig {
        schema: Some(&options),
        ..Default::default()
    });
    let keywords = keyword_completion_source(SqlDialect::StandardSql.language(), false);
    for position in [8, doc.text().len() + 1, usize::MAX] {
        assert!(schema.complete(&doc, position, true).is_none());
        assert!(keywords.complete(&doc, position, true).is_none());
    }
}
