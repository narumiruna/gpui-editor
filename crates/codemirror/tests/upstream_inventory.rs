//! Offline consistency checks. See the inventory for the archive and export checks.

use std::collections::{HashMap, HashSet};

use gpui_editor_codemirror::legacy_modes::LegacyMode;
use serde_json::{Value, json};

fn export_inventory() -> Value {
    serde_json::from_str(include_str!(
        "../../../docs/codemirror-upstream-exports.json"
    ))
    .unwrap()
}

#[derive(Default)]
struct Counts {
    modules: usize,
    specifiers: usize,
    exports: usize,
    exports_by_specifier: usize,
}

impl Counts {
    fn record(&mut self, specifiers: usize, exports: usize) {
        self.modules += 1;
        self.specifiers += specifiers;
        self.exports += exports;
        self.exports_by_specifier += exports * specifiers;
    }

    fn as_json(&self) -> Value {
        json!({
            "modules": self.modules,
            "specifiers": self.specifiers,
            "exports": self.exports,
            "exportsBySpecifier": self.exports_by_specifier,
        })
    }
}

#[test]
fn inventory_covers_every_pin_and_counts_entries_and_aliases() {
    let pins: Value =
        serde_json::from_str(include_str!("../../../docs/codemirror-upstream.json")).unwrap();
    let inventory = export_inventory();
    let mut counts = HashMap::<&str, Counts>::new();
    let mut specifiers = HashSet::new();
    let mut tarballs = 0;
    let mut archive_files = 0;

    for group in ["packages", "referenceDependencies"] {
        let pinned = pins[group].as_array().unwrap();
        let mut by_name: HashMap<_, _> = pinned
            .iter()
            .map(|pin| (pin["name"].as_str().unwrap(), pin))
            .collect();
        assert_eq!(by_name.len(), pinned.len(), "duplicate pin in {group}");
        for archive in inventory[group].as_array().unwrap() {
            let name = archive["name"].as_str().unwrap();
            let pin = by_name
                .remove(name)
                .unwrap_or_else(|| panic!("unexpected or duplicate archive: {name}"));
            assert_eq!(archive["version"], pin["version"], "{name}");
            assert_eq!(archive["tarball"], pin["tarball"], "{name}");
            assert_eq!(archive["verifiedIntegrity"], pin["integrity"], "{name}");
            let url = pin["tarball"].as_str().unwrap();
            assert_eq!(archive["artifact"], url.rsplit_once('/').unwrap().1);
            tarballs += 1;
            archive_files += archive["archiveFileCount"].as_u64().unwrap();

            let modules = archive["modules"].as_array().unwrap();
            assert!(!modules.is_empty(), "{name}");
            let mut files = HashSet::new();
            for module in modules {
                let file = module["file"].as_str().unwrap();
                assert!(files.insert(file), "duplicate module: {name} {file}");
                let aliases = module["specifiers"].as_array().unwrap();
                let category = match (name, file) {
                    ("@codemirror/legacy-modes", _) => {
                        let stem = file
                            .strip_prefix("package/mode/")
                            .and_then(|path| path.strip_suffix(".js"))
                            .unwrap();
                        assert!(!stem.is_empty() && !stem.contains('/'), "{file}");
                        assert_eq!(module["specifiers"], json!([format!("{name}/mode/{stem}")]));
                        "legacyModeSubpaths"
                    }
                    (_, "package/dist/index.js") => {
                        assert_eq!(module["specifiers"], json!([name]));
                        "packageRoots"
                    }
                    ("@lezer/generator", "package/dist/test.js") => {
                        assert_eq!(
                            module["specifiers"],
                            json!(["@lezer/generator/dist/test", "@lezer/generator/test"])
                        );
                        "distSubpaths"
                    }
                    ("@lezer/generator", "package/dist/rollup-plugin-lezer.js") => {
                        assert_eq!(module["specifiers"], json!(["@lezer/generator/rollup"]));
                        "distSubpaths"
                    }
                    _ => panic!("unexpected module: {name} {file}"),
                };
                for alias in aliases {
                    assert!(
                        specifiers.insert(alias.as_str().unwrap()),
                        "duplicate specifier: {alias}"
                    );
                }

                let exports: Vec<_> = module["exports"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|value| value.as_str().unwrap())
                    .collect();
                assert!(!exports.is_empty(), "no exports: {name} {file}");
                assert!(exports.iter().all(|name| !name.is_empty()));
                assert!(
                    exports.windows(2).all(|pair| pair[0] < pair[1]),
                    "unsorted or duplicate export: {name} {file}"
                );
                for key in [category, "total"] {
                    counts
                        .entry(key)
                        .or_default()
                        .record(aliases.len(), exports.len());
                }
            }
        }
        assert!(by_name.is_empty(), "missing pins in {group}: {by_name:?}");
    }

    assert_eq!(inventory["counts"]["tarballs"], tarballs);
    assert_eq!(inventory["counts"]["archiveFiles"], archive_files);
    for category in [
        "packageRoots",
        "distSubpaths",
        "legacyModeSubpaths",
        "total",
    ] {
        let observed = counts.get(category).unwrap();
        assert_eq!(
            inventory["counts"][category],
            observed.as_json(),
            "{category}"
        );
    }
}

#[test]
fn external_runtime_pins_do_not_change_migration_package_counts() {
    let pins: Value =
        serde_json::from_str(include_str!("../../../docs/codemirror-upstream.json")).unwrap();
    let mut names = HashSet::new();
    let runtime = pins["runtimeDependencies"].as_array().unwrap();
    assert_eq!(runtime.len(), 8);
    for group in ["packages", "referenceDependencies", "runtimeDependencies"] {
        for pin in pins[group].as_array().unwrap() {
            let name = pin["name"].as_str().unwrap();
            let version = pin["version"].as_str().unwrap();
            let basename = name.rsplit('/').next().unwrap();
            let integrity = pin["integrity"].as_str().unwrap();
            assert!(names.insert(name), "duplicate npm pin {name}");
            assert_eq!(
                pin["tarball"],
                format!("https://registry.npmjs.org/{name}/-/{basename}-{version}.tgz")
            );
            assert!(
                integrity.starts_with("sha512-") && integrity.len() == 95,
                "{name}: {integrity}"
            );
            assert_eq!(pin["license"], "MIT", "{name}");
        }
    }
    assert_eq!(pins["packages"].as_array().unwrap().len(), 37);
    assert_eq!(names.len(), 64);
}

#[test]
fn bundled_legacy_modes_match_unique_upstream_concrete_exports() {
    // Classification from the exact release's runtime values. The other
    // exports are seven factories and the CSS keyword table.
    const NOT_CONCRETE: &[(&str, &str)] = &[
        ("asn1", "asn1"),
        ("clike", "clike"),
        ("css", "keywords"),
        ("css", "mkCSS"),
        ("python", "mkPython"),
        ("simple-mode", "simpleMode"),
        ("sql", "sql"),
        ("xml", "mkXML"),
    ];
    let inventory = export_inventory();
    let archive = inventory["packages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|package| package["name"] == "@codemirror/legacy-modes")
        .unwrap();
    let mut total = 0;
    let mut bundled = HashSet::new();
    for module in archive["modules"].as_array().unwrap() {
        let path = module["specifiers"][0].as_str().unwrap();
        let stem = path.rsplit('/').next().unwrap();
        for export in module["exports"].as_array().unwrap() {
            let name = export.as_str().unwrap();
            let mode = LegacyMode::from_export(path, name);
            if NOT_CONCRETE.contains(&(stem, name)) {
                assert!(mode.is_none(), "{path}#{name} is not a concrete mode");
                continue;
            }
            total += 1;
            if let Some(mode) = mode {
                assert!(
                    bundled.insert(mode as usize),
                    "duplicate port: {path}#{name}"
                );
            }
        }
    }
    assert_eq!(total, 144);
    assert_eq!(bundled.len(), total, "missing concrete legacy exports");
    assert_eq!(
        bundled,
        LegacyMode::ALL
            .into_iter()
            .map(|mode| mode as usize)
            .collect()
    );
}
