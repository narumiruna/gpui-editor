//! Fixtures from the SRI-verified `@codemirror/language-data` 6.5.2 and
//! `@codemirror/language` 6.13.1 distributions in `docs/codemirror-upstream.json`.
//! The JS oracle ran `LanguageDescription.of`, `load`, `matchFilename`, and
//! `matchLanguageName`. It intercepted imports to record each module, export,
//! and argument list, without loading the parser packages. A null argument
//! list denotes a direct export. Match results use zero-based catalog IDs.

use std::{
    ptr,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use gpui_editor_codemirror::language_data::{
    LANGUAGES, LanguageDescription, LoadInvocation, LoadSource, NativeLanguage, NativeLoaders,
    NativeSyntax, UnavailableLanguage,
};
use gpui_editor_codemirror::{
    Text,
    language::{javascript::JavascriptOptions, sass::SassDialect, sql::SqlDialect},
    legacy_modes::LegacyMode,
};

const JS: NativeLanguage = NativeLanguage::Javascript(JavascriptOptions {
    jsx: false,
    typescript: false,
});
const JSX: NativeLanguage = NativeLanguage::Javascript(JavascriptOptions {
    jsx: true,
    typescript: false,
});
const TS: NativeLanguage = NativeLanguage::Javascript(JavascriptOptions {
    jsx: false,
    typescript: true,
});
const TSX: NativeLanguage = NativeLanguage::Javascript(JavascriptOptions {
    jsx: true,
    typescript: true,
});
use serde_json::{Value, json};

fn oracle() -> Value {
    serde_json::from_str(include_str!("../src/language_data/oracle.json")).unwrap()
}

fn source_id(source: LoadSource) -> Value {
    let args = match source.invocation {
        LoadInvocation::Export => Value::Null,
        LoadInvocation::Call => json!([]),
        LoadInvocation::EmptyOptions => json!([{}]),
        LoadInvocation::JavaScript { jsx, typescript } => {
            let mut options = json!({});
            if jsx {
                options["jsx"] = json!(true);
            }
            if typescript {
                options["typescript"] = json!(true);
            }
            json!([options])
        }
        LoadInvocation::SassIndented => json!([{ "indented": true }]),
        LoadInvocation::SqlDialect(export) => json!([{ "dialect": [source.module, export, null] }]),
    };
    json!([source.module, source.export, args])
}

fn match_id(catalog: &[LanguageDescription], matched: Option<&LanguageDescription>) -> Value {
    json!(matched.map(|language| {
        catalog
            .iter()
            .position(|candidate| ptr::eq(candidate, language))
            .expect("match must refer to an item in the catalog")
    }))
}

#[test]
fn matches_all_ordered_catalog_metadata_and_loader_ids() {
    let oracle = oracle();
    let catalog = oracle["catalog"].as_array().unwrap();
    assert_eq!(LANGUAGES.len(), catalog.len());
    for (id, (description, expected)) in LANGUAGES.iter().zip(catalog).enumerate() {
        assert_eq!(
            json!([
                description.name,
                description.aliases,
                description.extensions,
                description.filename.map(|re| [re.source(), re.flags()]),
                source_id(description.source),
            ]),
            *expected,
            "catalog ID {id}",
        );
    }
}

#[test]
fn matches_filenames_including_all_catalog_extensions() {
    for row in oracle()["filenames"].as_array().unwrap() {
        let filename = row[0].as_str().unwrap();
        let matched = LanguageDescription::match_filename(LANGUAGES, filename);
        assert_eq!(
            match_id(LANGUAGES, matched),
            row[1],
            "filename {filename:?}"
        );
    }
}

#[test]
fn matches_filename_patterns_before_any_extension() {
    let asterisk = LanguageDescription::match_name(LANGUAGES, "Asterisk", false).unwrap();
    let catalog = [
        LanguageDescription {
            name: "Extension",
            aliases: &["extension"],
            extensions: &["conf"],
            ..LANGUAGES[0]
        },
        *asterisk,
    ];
    for row in oracle()["patternPriority"].as_array().unwrap() {
        let filename = row[0].as_str().unwrap();
        let matched = LanguageDescription::match_filename(&catalog, filename);
        assert_eq!(match_id(&catalog, matched), row[1], "filename {filename:?}");
    }
}

#[test]
fn matches_all_exact_aliases_and_fuzzy_name_cases() {
    for row in oracle()["names"].as_array().unwrap() {
        let name = row[0].as_str().unwrap();
        let fuzzy = row[1].as_bool().unwrap();
        let matched = LanguageDescription::match_name(LANGUAGES, name, fuzzy);
        assert_eq!(
            match_id(LANGUAGES, matched),
            row[2],
            "name {name:?}, fuzzy {fuzzy}"
        );
    }
}

#[test]
fn matches_custom_names_with_upstream_aliases_and_utf16_boundaries() {
    // These are the original custom specs in fixtures/language_data_oracle.mjs.
    let catalog = [
        ("Tool", &["GO", "𐐀x"][..]),
        ("(Go)", &[][..]),
        ("Go", &[][..]),
        ("😀", &[][..]),
        ("ΣΙΓΜΑΣ", &["ΟΣ"][..]),
        ("İ", &["I"][..]),
        ("Empty", &[""][..]),
    ]
    .map(|(name, aliases)| LanguageDescription {
        name,
        aliases,
        ..LANGUAGES[0]
    });
    for line in include_str!("fixtures/language_data_custom_names.jsonl").lines() {
        let (name, fuzzy, expected): (String, bool, Option<usize>) =
            serde_json::from_str(line).unwrap();
        let matched = LanguageDescription::match_name(&catalog, &name, fuzzy);
        assert_eq!(
            match_id(&catalog, matched),
            json!(expected),
            "name {name:?}, fuzzy {fuzzy}"
        );
    }
}

#[test]
fn resolves_and_uses_each_bundled_parser() {
    for (filename, language, source, tag) in [
        (
            "style.css",
            NativeLanguage::Css,
            ".notice { color: red; }\r\n",
            "propertyName",
        ),
        (
            "main.go",
            NativeLanguage::Go,
            "package main\r\nfunc f() string { return \"字\" }\r\n",
            "definitionKeyword",
        ),
        (
            "index.html",
            NativeLanguage::Html,
            "<style>\r\na { color: red; }\r\n</style>\r\n",
            "propertyName",
        ),
        (
            "README.md",
            NativeLanguage::Markdown,
            "# 字\r\n",
            "heading1",
        ),
        ("sample.c", NativeLanguage::Cpp, "int i = 42;\r\n", "number"),
        (
            "sample.cpp",
            NativeLanguage::Cpp,
            "auto s = \"字\";\r\n",
            "string",
        ),
        (
            "Widget.java",
            NativeLanguage::Java,
            "class Widget { String s = \"字\"; }\r\n",
            "string",
        ),
        (
            "theme.less",
            NativeLanguage::Less,
            ".card { color: red; }\r\n",
            "propertyName",
        ),
        (
            "page.liquid",
            NativeLanguage::Liquid,
            "<a>{{ product.title }}</a>\r\n",
            "tagName",
        ),
        (
            "page.php",
            NativeLanguage::Php,
            "<a><?= $caption ?></a>\r\n",
            "tagName",
        ),
        (
            "theme.sass",
            NativeLanguage::Sass(SassDialect::Indented),
            ".card\r\n  color: red\r\n",
            "propertyName",
        ),
        (
            "theme.scss",
            NativeLanguage::Sass(SassDialect::Scss),
            ".card { color: red; }\r\n",
            "propertyName",
        ),
        (
            "Card.vue",
            NativeLanguage::Vue,
            "<template><main>{{ name }}</main></template>\r\n",
            "variableName",
        ),
        (
            "page.j2",
            NativeLanguage::Jinja,
            "<p>{{ title }}</p>\r\n",
            "tagName",
        ),
        ("app.mjs", JS, "const name = \"字\";\r\n", "string"),
        ("app.jsx", JSX, "const view = <p>字</p>;\r\n", "tagName"),
        ("app.ts", TS, "const name: string = \"字\";\r\n", "string"),
        ("app.tsx", TSX, "const view = <p>字</p>;\r\n", "tagName"),
        (
            "db.sql",
            NativeLanguage::Sql(SqlDialect::StandardSql),
            "SELECT '字';\r\n",
            "keyword",
        ),
        (
            "main.py",
            NativeLanguage::Python,
            "def f():\r\n    return \"字\"\r\n",
            "string",
        ),
        (
            "main.rs",
            NativeLanguage::Rust,
            "fn main() {}\r\n",
            "definitionKeyword",
        ),
        (
            "settings.json",
            NativeLanguage::Json,
            "{\"字\":true}\r\n",
            "propertyName",
        ),
        (
            "module.wat",
            NativeLanguage::Wast,
            "(module)\r\n",
            "keyword",
        ),
        (
            "config.xml",
            NativeLanguage::Xml,
            "<a>字</a>\r\n",
            "tagName",
        ),
        (
            "config.yaml",
            NativeLanguage::Yaml,
            "name: \"字\"\r\n",
            "string",
        ),
    ] {
        let description = LanguageDescription::match_filename(LANGUAGES, filename).unwrap();
        assert_eq!(description.built_in(), Some(language));
        let parsed = language.parse(Text::new(source)).unwrap();
        assert_eq!(parsed.text().as_str(), source);
        assert!(parsed.highlights(0..source.len()).unwrap().any(|span| {
            parsed.text().slice(span.range).is_some()
                && span.tags.iter().any(|item| item.name() == tag)
        }));
        if let NativeSyntax::Json(json) = parsed {
            assert_eq!(
                json.root().children().next().unwrap().kind().name(),
                "Object"
            );
            assert!(json.diagnostic().is_none());
        }
    }
    let angular = LanguageDescription::match_name(LANGUAGES, "Angular Template", false)
        .and_then(LanguageDescription::built_in)
        .unwrap();
    assert_eq!(angular, NativeLanguage::Angular);
    let parsed = angular
        .parse(Text::new("<card [title]=\"name\"/>\r\n"))
        .unwrap();
    assert!(
        parsed
            .highlights(0..parsed.text().len())
            .unwrap()
            .any(|span| {
                parsed.text().slice(span.range) == Some("name")
                    && span.tags[0].name() == "variableName"
            })
    );
    let available: Vec<_> = LANGUAGES
        .iter()
        .filter_map(LanguageDescription::built_in)
        .collect();
    assert_eq!(
        available,
        [
            NativeLanguage::Cpp,
            NativeLanguage::Cpp,
            NativeLanguage::Sql(SqlDialect::Cassandra),
            NativeLanguage::Css,
            NativeLanguage::Go,
            NativeLanguage::Html,
            NativeLanguage::Java,
            JS,
            NativeLanguage::Jinja,
            NativeLanguage::Json,
            JSX,
            NativeLanguage::Less,
            NativeLanguage::Liquid,
            NativeLanguage::Sql(SqlDialect::MariaSql),
            NativeLanguage::Markdown,
            NativeLanguage::Sql(SqlDialect::MsSql),
            NativeLanguage::Sql(SqlDialect::MySql),
            NativeLanguage::Php,
            NativeLanguage::Sql(SqlDialect::PlSql),
            NativeLanguage::Sql(SqlDialect::PostgreSql),
            NativeLanguage::Python,
            NativeLanguage::Rust,
            NativeLanguage::Sass(SassDialect::Indented),
            NativeLanguage::Sass(SassDialect::Scss),
            NativeLanguage::Sql(SqlDialect::StandardSql),
            NativeLanguage::Sql(SqlDialect::Sqlite),
            TSX,
            TS,
            NativeLanguage::Wast,
            NativeLanguage::Xml,
            NativeLanguage::Yaml,
            NativeLanguage::Legacy(LegacyMode::Apl),
            NativeLanguage::Legacy(LegacyMode::AsciiArmor),
            NativeLanguage::Asn1,
            NativeLanguage::Legacy(LegacyMode::Asterisk),
            NativeLanguage::Legacy(LegacyMode::Brainfuck),
            NativeLanguage::Legacy(LegacyMode::Cobol),
            NativeLanguage::Legacy(LegacyMode::CSharp),
            NativeLanguage::Legacy(LegacyMode::Clojure),
            NativeLanguage::Legacy(LegacyMode::Clojure),
            NativeLanguage::Legacy(LegacyMode::Gss),
            NativeLanguage::Legacy(LegacyMode::CMake),
            NativeLanguage::Legacy(LegacyMode::CoffeeScript),
            NativeLanguage::Legacy(LegacyMode::CommonLisp),
            NativeLanguage::Legacy(LegacyMode::Cypher),
            NativeLanguage::Legacy(LegacyMode::Cython),
            NativeLanguage::Legacy(LegacyMode::Crystal),
            NativeLanguage::Legacy(LegacyMode::D),
            NativeLanguage::Legacy(LegacyMode::Dart),
            NativeLanguage::Legacy(LegacyMode::Diff),
            NativeLanguage::Legacy(LegacyMode::Dockerfile),
            NativeLanguage::Legacy(LegacyMode::Dtd),
            NativeLanguage::Legacy(LegacyMode::Dylan),
            NativeLanguage::Legacy(LegacyMode::Ebnf),
            NativeLanguage::Legacy(LegacyMode::Ecl),
            NativeLanguage::Legacy(LegacyMode::Clojure),
            NativeLanguage::Legacy(LegacyMode::Eiffel),
            NativeLanguage::Legacy(LegacyMode::Elm),
            NativeLanguage::Legacy(LegacyMode::Erlang),
            NativeLanguage::Legacy(LegacyMode::Esper),
            NativeLanguage::Legacy(LegacyMode::Factor),
            NativeLanguage::Legacy(LegacyMode::Fcl),
            NativeLanguage::Legacy(LegacyMode::Forth),
            NativeLanguage::Legacy(LegacyMode::Fortran),
            NativeLanguage::Legacy(LegacyMode::FSharp),
            NativeLanguage::Legacy(LegacyMode::Gas),
            NativeLanguage::Legacy(LegacyMode::Gherkin),
            NativeLanguage::Legacy(LegacyMode::Groovy),
            NativeLanguage::Legacy(LegacyMode::Haskell),
            NativeLanguage::Legacy(LegacyMode::Haxe),
            NativeLanguage::Legacy(LegacyMode::Hxml),
            NativeLanguage::Legacy(LegacyMode::Http),
            NativeLanguage::Legacy(LegacyMode::Idl),
            NativeLanguage::Legacy(LegacyMode::JsonLd),
            NativeLanguage::Legacy(LegacyMode::Julia),
            NativeLanguage::Legacy(LegacyMode::Kotlin),
            NativeLanguage::Legacy(LegacyMode::LiveScript),
            NativeLanguage::Legacy(LegacyMode::Lua),
            NativeLanguage::Legacy(LegacyMode::Mirc),
            NativeLanguage::Legacy(LegacyMode::Mathematica),
            NativeLanguage::Legacy(LegacyMode::Modelica),
            NativeLanguage::Legacy(LegacyMode::Mumps),
            NativeLanguage::Legacy(LegacyMode::Mbox),
            NativeLanguage::Legacy(LegacyMode::Nginx),
            NativeLanguage::Legacy(LegacyMode::Nsis),
            NativeLanguage::Legacy(LegacyMode::NTriples),
            NativeLanguage::Legacy(LegacyMode::ObjectiveC),
            NativeLanguage::Legacy(LegacyMode::ObjectiveCpp),
            NativeLanguage::Legacy(LegacyMode::OCaml),
            NativeLanguage::Legacy(LegacyMode::Octave),
            NativeLanguage::Legacy(LegacyMode::Oz),
            NativeLanguage::Legacy(LegacyMode::Pascal),
            NativeLanguage::Legacy(LegacyMode::Perl),
            NativeLanguage::Legacy(LegacyMode::Pig),
            NativeLanguage::Legacy(LegacyMode::PowerShell),
            NativeLanguage::Legacy(LegacyMode::Properties),
            NativeLanguage::Legacy(LegacyMode::Protobuf),
            NativeLanguage::Legacy(LegacyMode::Pug),
            NativeLanguage::Legacy(LegacyMode::Puppet),
            NativeLanguage::Legacy(LegacyMode::Q),
            NativeLanguage::Legacy(LegacyMode::R),
            NativeLanguage::Legacy(LegacyMode::RpmChanges),
            NativeLanguage::Legacy(LegacyMode::RpmSpec),
            NativeLanguage::Legacy(LegacyMode::Ruby),
            NativeLanguage::Legacy(LegacyMode::Sas),
            NativeLanguage::Legacy(LegacyMode::Scala),
            NativeLanguage::Legacy(LegacyMode::Scheme),
            NativeLanguage::Legacy(LegacyMode::Shell),
            NativeLanguage::Legacy(LegacyMode::Sieve),
            NativeLanguage::Legacy(LegacyMode::Smalltalk),
            NativeLanguage::Legacy(LegacyMode::Solr),
            NativeLanguage::Legacy(LegacyMode::Sml),
            NativeLanguage::Legacy(LegacyMode::Sparql),
            NativeLanguage::Legacy(LegacyMode::Spreadsheet),
            NativeLanguage::Legacy(LegacyMode::Squirrel),
            NativeLanguage::Legacy(LegacyMode::Stylus),
            NativeLanguage::Legacy(LegacyMode::Swift),
            NativeLanguage::Legacy(LegacyMode::Stex),
            NativeLanguage::Legacy(LegacyMode::Stex),
            NativeLanguage::Legacy(LegacyMode::Verilog),
            NativeLanguage::Legacy(LegacyMode::Tcl),
            NativeLanguage::Legacy(LegacyMode::Textile),
            NativeLanguage::Legacy(LegacyMode::TiddlyWiki),
            NativeLanguage::Legacy(LegacyMode::Tiki),
            NativeLanguage::Legacy(LegacyMode::Toml),
            NativeLanguage::Legacy(LegacyMode::Troff),
            NativeLanguage::Legacy(LegacyMode::Ttcn),
            NativeLanguage::Legacy(LegacyMode::TtcnCfg),
            NativeLanguage::Legacy(LegacyMode::Turtle),
            NativeLanguage::Legacy(LegacyMode::WebIdl),
            NativeLanguage::Legacy(LegacyMode::Vb),
            NativeLanguage::Legacy(LegacyMode::VbScript),
            NativeLanguage::Legacy(LegacyMode::Velocity),
            NativeLanguage::Legacy(LegacyMode::Verilog),
            NativeLanguage::Legacy(LegacyMode::Vhdl),
            NativeLanguage::Legacy(LegacyMode::XQuery),
            NativeLanguage::Legacy(LegacyMode::Yacas),
            NativeLanguage::Legacy(LegacyMode::Z80),
            NativeLanguage::Legacy(LegacyMode::Mscgen),
            NativeLanguage::Legacy(LegacyMode::Xu),
            NativeLanguage::Legacy(LegacyMode::Msgenny),
            NativeLanguage::Vue,
            NativeLanguage::Angular,
        ]
    );
}

#[test]
fn every_published_catalog_entry_resolves_and_runs_natively() {
    assert_eq!(LANGUAGES.len(), 143);
    const SOURCE: &str = "x\r\n";
    for entry in LANGUAGES {
        let handle = entry.native_load();
        assert!(handle.support().is_none(), "{}", entry.name);
        let language = handle.load().unwrap_or_else(|error| panic!("{error}"));
        assert!(
            Arc::ptr_eq(&language, &handle.support().unwrap()),
            "{}",
            entry.name
        );
        assert!(
            Arc::ptr_eq(&language, &handle.load().unwrap()),
            "{}",
            entry.name
        );
        let parsed = language
            .parse(Text::new(SOURCE))
            .unwrap_or_else(|error| panic!("{}: {error}", entry.name));
        assert_eq!(parsed.text().as_str(), SOURCE, "{}", entry.name);
        let spans = parsed
            .highlights(0..SOURCE.len())
            .unwrap_or_else(|| panic!("{} returned no highlight data", entry.name));
        for span in spans {
            assert!(parsed.text().slice(span.range).is_some(), "{}", entry.name);
        }
    }
}

#[test]
fn descriptions_with_the_same_recipe_have_independent_load_state() {
    let (c, cpp) = (&LANGUAGES[0], &LANGUAGES[1]);
    assert_eq!(c.source, cpp.source);
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    let mut registry = NativeLoaders::default();
    registry.register(c.source, move || counter.fetch_add(1, Ordering::SeqCst));
    let a = registry.cached(c);
    let b = registry.cached(cpp);
    assert_eq!(a.description().name, c.name);
    assert_eq!(*a.load().unwrap(), 0);
    assert!(b.support().is_none());
    assert_eq!(*b.load().unwrap(), 1);
    assert_eq!(*a.load().unwrap(), 0);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[test]
fn bundled_legacy_modes_use_the_exact_catalog_sources() {
    for (id, mode) in [
        (32, LegacyMode::AsciiArmor),
        (35, LegacyMode::Brainfuck),
        (49, LegacyMode::Diff),
        (71, LegacyMode::Http),
        (95, LegacyMode::Properties),
        (96, LegacyMode::Protobuf),
        (124, LegacyMode::Toml),
    ] {
        assert_eq!(LANGUAGES[id].built_in(), Some(NativeLanguage::Legacy(mode)));
    }
    let doc = LanguageDescription::match_filename(LANGUAGES, "patch.diff")
        .and_then(LanguageDescription::built_in)
        .unwrap()
        .parse(Text::new("-字\r\n+好\r\n"))
        .unwrap();
    let tags: Vec<_> = doc
        .highlights(0..doc.text().len())
        .unwrap()
        .flat_map(|span| span.tags.iter().map(|tag| tag.name()))
        .collect();
    assert_eq!(tags, ["deleted", "inserted"]);
}

#[test]
fn custom_loader_registry_starts_empty() {
    let loaders = NativeLoaders::<()>::default();
    for language in LANGUAGES {
        assert!(!loaders.is_available(language));
        assert_eq!(
            loaders.load(language),
            Err(UnavailableLanguage {
                name: language.name,
                source: language.source,
            }),
        );
    }
}

#[test]
fn calls_only_the_registered_factory_for_the_exact_source() {
    let find = |name| LanguageDescription::match_name(LANGUAGES, name, false).unwrap();
    let js = find("JavaScript");
    let jsx = find("JSX");
    let c = find("C");
    let cpp = find("C++");
    let calls = Arc::new(AtomicUsize::new(0));
    let mut loaders = NativeLoaders::default();
    loaders.register(js.source, || 10);
    loaders.register(c.source, {
        let calls = Arc::clone(&calls);
        move || calls.fetch_add(1, Ordering::Relaxed)
    });

    assert_eq!(loaders.load(js), Ok(10));
    assert!(!loaders.is_available(jsx));
    assert!(loaders.load(jsx).is_err());
    assert_eq!(loaders.load(c), Ok(0));
    assert!(loaders.is_available(cpp));
    assert_eq!(loaders.load(cpp), Ok(1));
    assert_eq!(calls.load(Ordering::Relaxed), 2);
}
