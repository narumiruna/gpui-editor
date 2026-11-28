// Generated from the SRI-verified @codemirror/language-data 6.5.2 dist/index.js.
// See docs/codemirror-upstream.json and docs/licenses/codemirror-language-data.txt.

use std::{future::ready, ptr, sync::OnceLock};

use crate::Extension;

use super::{
    FilenamePattern, LanguageDescription, LoadInvocation::*, LoadSource, NativeAsyncLanguageLoad,
    NativeLanguageSupport, UnavailableLanguage,
};

type SupportLoad = NativeAsyncLanguageLoad<'static, NativeLanguageSupport, UnavailableLanguage>;

impl LanguageDescription {
    /// Get the shared async load handle for this [`LANGUAGES`] entry.
    /// This caches the attempt and its result for the life of the catalog.
    /// Return `None` for custom descriptions and copies of catalog entries.
    pub fn catalog_support_load(&self) -> Option<&'static SupportLoad> {
        // Cache by catalog position, not by recipe. Multiple descriptions
        // can use the same recipe without sharing their loading state.
        static LOADS: [OnceLock<SupportLoad>; LANGUAGES.len()] =
            [const { OnceLock::new() }; LANGUAGES.len()];
        let id = LANGUAGES.iter().position(|entry| ptr::eq(entry, self))?;
        Some(LOADS[id].get_or_init(|| {
            NativeAsyncLanguageLoad::new(&LANGUAGES[id], move |description| {
                ready(
                    description
                        .built_in()
                        .map(|language| NativeLanguageSupport::for_catalog(language, id))
                        .ok_or(UnavailableLanguage {
                            name: description.name,
                            source: description.source,
                        }),
                )
            })
        }))
    }
}

pub(super) fn stream_extension(id: usize, support: NativeLanguageSupport) -> Extension {
    static EXTENSIONS: [OnceLock<Extension>; LANGUAGES.len()] =
        [const { OnceLock::new() }; LANGUAGES.len()];
    EXTENSIONS[id]
        .get_or_init(|| support.tracked_syntax_extension())
        .clone()
}

macro_rules! package {
    ($name:literal, $export:literal) => {
        package!($name, $export, Call)
    };
    ($name:literal, $export:literal, $invocation:expr) => {
        LoadSource {
            module: concat!("@codemirror/lang-", $name),
            export: $export,
            invocation: $invocation,
        }
    };
}

macro_rules! legacy {
    ($mode:literal, $export:literal) => {
        legacy!($mode, $export, Export)
    };
    ($mode:literal, $export:literal, $invocation:expr) => {
        LoadSource {
            module: concat!("@codemirror/legacy-modes/mode/", $mode),
            export: $export,
            invocation: $invocation,
        }
    };
}

// For these ASCII literals, JavaScript `i` without `u` uses ASCII case
// folding. JavaScript `.` excludes CR, LF, U+2028, and U+2029.
macro_rules! filename {
    ($source:literal) => {
        filename!($source, "", $source)
    };
    ($source:literal, $flags:literal, $rust:literal) => {{
        static REGEX: std::sync::LazyLock<regex::Regex> =
            std::sync::LazyLock::new(|| regex::Regex::new($rust).expect("valid filename pattern"));
        Some(FilenamePattern {
            source: $source,
            flags: $flags,
            regex: &REGEX,
        })
    }};
}

const fn description(
    name: &'static str,
    aliases: &'static [&'static str],
    extensions: &'static [&'static str],
    filename: Option<FilenamePattern>,
    source: LoadSource,
) -> LanguageDescription {
    LanguageDescription {
        name,
        aliases,
        extensions,
        filename,
        source,
    }
}

/// All language descriptions in upstream priority order.
#[rustfmt::skip]
pub static LANGUAGES: &[LanguageDescription] = &[
    description("C", &["c"], &["c", "h", "ino"], None, package!("cpp", "cpp")),
    description("C++", &["cpp", "c++"], &["cpp", "c++", "cc", "cxx", "hpp", "h++", "hh", "hxx"], None, package!("cpp", "cpp")),
    description("CQL", &["cassandra", "cql"], &["cql"], None, package!("sql", "sql", SqlDialect("Cassandra"))),
    description("CSS", &["css"], &["css"], None, package!("css", "css")),
    description("Go", &["go"], &["go"], None, package!("go", "go")),
    description("HTML", &["xhtml", "html"], &["html", "htm", "handlebars", "hbs"], None, package!("html", "html")),
    description("Java", &["java"], &["java"], None, package!("java", "java")),
    description("JavaScript", &["ecmascript", "js", "node", "javascript"], &["js", "mjs", "cjs"], None, package!("javascript", "javascript")),
    description("Jinja", &["jinja"], &["j2", "jinja", "jinja2"], None, package!("jinja", "jinja")),
    description("JSON", &["json5", "json"], &["json", "map"], None, package!("json", "json")),
    description("JSX", &["jsx"], &["jsx"], None, package!("javascript", "javascript", JavaScript { jsx: true, typescript: false })),
    description("LESS", &["less"], &["less"], None, package!("less", "less")),
    description("Liquid", &["liquid"], &["liquid"], None, package!("liquid", "liquid")),
    description("MariaDB SQL", &["mariadb sql"], &[], None, package!("sql", "sql", SqlDialect("MariaSQL"))),
    description("Markdown", &["markdown"], &["md", "markdown", "mkd"], None, package!("markdown", "markdown")),
    description("MS SQL", &["ms sql"], &[], None, package!("sql", "sql", SqlDialect("MSSQL"))),
    description("MySQL", &["mysql"], &[], None, package!("sql", "sql", SqlDialect("MySQL"))),
    description("PHP", &["php"], &["php", "php3", "php4", "php5", "php7", "phtml"], None, package!("php", "php")),
    description("PLSQL", &["plsql"], &["pls"], None, package!("sql", "sql", SqlDialect("PLSQL"))),
    description("PostgreSQL", &["postgresql"], &[], None, package!("sql", "sql", SqlDialect("PostgreSQL"))),
    description("Python", &["python"], &["BUILD", "bzl", "py", "pyw"], filename!(r"^(BUCK|BUILD)$"), package!("python", "python")),
    description("Rust", &["rust"], &["rs"], None, package!("rust", "rust")),
    description("Sass", &["sass"], &["sass"], None, package!("sass", "sass", SassIndented)),
    description("SCSS", &["scss"], &["scss"], None, package!("sass", "sass")),
    description("SQL", &["sql"], &["sql"], None, package!("sql", "sql", SqlDialect("StandardSQL"))),
    description("SQLite", &["sqlite"], &[], None, package!("sql", "sql", SqlDialect("SQLite"))),
    description("TSX", &["tsx"], &["tsx"], None, package!("javascript", "javascript", JavaScript { jsx: true, typescript: true })),
    description("TypeScript", &["ts", "typescript"], &["ts", "mts", "cts"], None, package!("javascript", "javascript", JavaScript { jsx: false, typescript: true })),
    description("WebAssembly", &["webassembly"], &["wat", "wast"], None, package!("wast", "wast")),
    description("XML", &["rss", "wsdl", "xsd", "xml"], &["xml", "xsl", "xsd", "svg"], None, package!("xml", "xml")),
    description("YAML", &["yml", "yaml"], &["yaml", "yml"], None, package!("yaml", "yaml")),
    description("APL", &["apl"], &["dyalog", "apl"], None, legacy!("apl", "apl")),
    description("PGP", &["asciiarmor", "pgp"], &["asc", "pgp", "sig"], None, legacy!("asciiarmor", "asciiArmor")),
    description("ASN.1", &["asn.1"], &["asn", "asn1"], None, legacy!("asn1", "asn1", EmptyOptions)),
    description("Asterisk", &["asterisk"], &[], filename!(r"^extensions\.conf$", "i", r"(?i-u:^extensions\.conf$)"), legacy!("asterisk", "asterisk")),
    description("Brainfuck", &["brainfuck"], &["b", "bf"], None, legacy!("brainfuck", "brainfuck")),
    description("Cobol", &["cobol"], &["cob", "cpy"], None, legacy!("cobol", "cobol")),
    description("C#", &["csharp", "cs", "c#"], &["cs"], None, legacy!("clike", "csharp")),
    description("Clojure", &["clojure"], &["clj", "cljc", "cljx"], None, legacy!("clojure", "clojure")),
    description("ClojureScript", &["clojurescript"], &["cljs"], None, legacy!("clojure", "clojure")),
    description("Closure Stylesheets (GSS)", &["closure stylesheets (gss)"], &["gss"], None, legacy!("css", "gss")),
    description("CMake", &["cmake"], &["cmake", "cmake.in"], filename!(r"^CMakeLists\.txt$"), legacy!("cmake", "cmake")),
    description("CoffeeScript", &["coffee", "coffee-script", "coffeescript"], &["coffee"], None, legacy!("coffeescript", "coffeeScript")),
    description("Common Lisp", &["lisp", "common lisp"], &["cl", "lisp", "el"], None, legacy!("commonlisp", "commonLisp")),
    description("Cypher", &["cypher"], &["cyp", "cypher"], None, legacy!("cypher", "cypher")),
    description("Cython", &["cython"], &["pyx", "pxd", "pxi"], None, legacy!("python", "cython")),
    description("Crystal", &["crystal"], &["cr"], None, legacy!("crystal", "crystal")),
    description("D", &["d"], &["d"], None, legacy!("d", "d")),
    description("Dart", &["dart"], &["dart"], None, legacy!("clike", "dart")),
    description("diff", &["diff"], &["diff", "patch"], None, legacy!("diff", "diff")),
    description("Dockerfile", &["dockerfile"], &[], filename!(r"^Dockerfile$"), legacy!("dockerfile", "dockerFile")),
    description("DTD", &["dtd"], &["dtd"], None, legacy!("dtd", "dtd")),
    description("Dylan", &["dylan"], &["dylan", "dyl", "intr"], None, legacy!("dylan", "dylan")),
    description("EBNF", &["ebnf"], &[], None, legacy!("ebnf", "ebnf")),
    description("ECL", &["ecl"], &["ecl"], None, legacy!("ecl", "ecl")),
    description("edn", &["edn"], &["edn"], None, legacy!("clojure", "clojure")),
    description("Eiffel", &["eiffel"], &["e"], None, legacy!("eiffel", "eiffel")),
    description("Elm", &["elm"], &["elm"], None, legacy!("elm", "elm")),
    description("Erlang", &["erlang"], &["erl"], None, legacy!("erlang", "erlang")),
    description("Esper", &["esper"], &[], None, legacy!("sql", "esper")),
    description("Factor", &["factor"], &["factor"], None, legacy!("factor", "factor")),
    description("FCL", &["fcl"], &[], None, legacy!("fcl", "fcl")),
    description("Forth", &["forth"], &["forth", "fth", "4th"], None, legacy!("forth", "forth")),
    description("Fortran", &["fortran"], &["f", "for", "f77", "f90", "f95"], None, legacy!("fortran", "fortran")),
    description("F#", &["fsharp", "f#"], &["fs"], None, legacy!("mllike", "fSharp")),
    description("Gas", &["gas"], &["s"], None, legacy!("gas", "gas")),
    description("Gherkin", &["gherkin"], &["feature"], None, legacy!("gherkin", "gherkin")),
    description("Groovy", &["groovy"], &["groovy", "gradle"], filename!(r"^Jenkinsfile$"), legacy!("groovy", "groovy")),
    description("Haskell", &["haskell"], &["hs"], None, legacy!("haskell", "haskell")),
    description("Haxe", &["haxe"], &["hx"], None, legacy!("haxe", "haxe")),
    description("HXML", &["hxml"], &["hxml"], None, legacy!("haxe", "hxml")),
    description("HTTP", &["http"], &[], None, legacy!("http", "http")),
    description("IDL", &["idl"], &["pro"], None, legacy!("idl", "idl")),
    description("JSON-LD", &["jsonld", "json-ld"], &["jsonld"], None, legacy!("javascript", "jsonld")),
    description("Julia", &["julia"], &["jl"], None, legacy!("julia", "julia")),
    description("Kotlin", &["kotlin"], &["kt", "kts"], None, legacy!("clike", "kotlin")),
    description("LiveScript", &["ls", "livescript"], &["ls"], None, legacy!("livescript", "liveScript")),
    description("Lua", &["lua"], &["lua"], None, legacy!("lua", "lua")),
    description("mIRC", &["mirc"], &["mrc"], None, legacy!("mirc", "mirc")),
    description("Mathematica", &["mathematica"], &["m", "nb", "wl", "wls"], None, legacy!("mathematica", "mathematica")),
    description("Modelica", &["modelica"], &["mo"], None, legacy!("modelica", "modelica")),
    description("MUMPS", &["mumps"], &["mps"], None, legacy!("mumps", "mumps")),
    description("Mbox", &["mbox"], &["mbox"], None, legacy!("mbox", "mbox")),
    description("Nginx", &["nginx"], &[], filename!(r"nginx.*\.conf$", "i", r"(?i-u:nginx)[^\n\r\u{2028}\u{2029}]*(?i-u:\.conf)$"), legacy!("nginx", "nginx")),
    description("NSIS", &["nsis"], &["nsh", "nsi"], None, legacy!("nsis", "nsis")),
    description("NTriples", &["ntriples"], &["nt", "nq"], None, legacy!("ntriples", "ntriples")),
    description("Objective-C", &["objective-c", "objc", "objective-c"], &["m"], None, legacy!("clike", "objectiveC")),
    description("Objective-C++", &["objective-c++", "objc++", "objective-c++"], &["mm"], None, legacy!("clike", "objectiveCpp")),
    description("OCaml", &["ocaml"], &["ml", "mli", "mll", "mly"], None, legacy!("mllike", "oCaml")),
    description("Octave", &["octave"], &["m"], None, legacy!("octave", "octave")),
    description("Oz", &["oz"], &["oz"], None, legacy!("oz", "oz")),
    description("Pascal", &["pascal"], &["p", "pas"], None, legacy!("pascal", "pascal")),
    description("Perl", &["perl"], &["pl", "pm"], None, legacy!("perl", "perl")),
    description("Pig", &["pig"], &["pig"], None, legacy!("pig", "pig")),
    description("PowerShell", &["powershell"], &["ps1", "psd1", "psm1"], None, legacy!("powershell", "powerShell")),
    description("Properties files", &["ini", "properties", "properties files"], &["properties", "ini", "in"], None, legacy!("properties", "properties")),
    description("ProtoBuf", &["protobuf"], &["proto"], None, legacy!("protobuf", "protobuf")),
    description("Pug", &["jade", "pug"], &["pug", "jade"], None, legacy!("pug", "pug")),
    description("Puppet", &["puppet"], &["pp"], None, legacy!("puppet", "puppet")),
    description("Q", &["q"], &["q"], None, legacy!("q", "q")),
    description("R", &["rscript", "r"], &["r", "R"], None, legacy!("r", "r")),
    description("RPM Changes", &["rpm changes"], &[], None, legacy!("rpm", "rpmChanges")),
    description("RPM Spec", &["rpm spec"], &["spec"], None, legacy!("rpm", "rpmSpec")),
    description("Ruby", &["jruby", "macruby", "rake", "rb", "rbx", "ruby"], &["rb"], filename!(r"^(Gemfile|Rakefile)$"), legacy!("ruby", "ruby")),
    description("SAS", &["sas"], &["sas"], None, legacy!("sas", "sas")),
    description("Scala", &["scala"], &["scala"], None, legacy!("clike", "scala")),
    description("Scheme", &["scheme"], &["scm", "ss"], None, legacy!("scheme", "scheme")),
    description("Shell", &["bash", "sh", "zsh", "shell"], &["sh", "ksh", "bash"], filename!(r"^PKGBUILD$"), legacy!("shell", "shell")),
    description("Sieve", &["sieve"], &["siv", "sieve"], None, legacy!("sieve", "sieve")),
    description("Smalltalk", &["smalltalk"], &["st"], None, legacy!("smalltalk", "smalltalk")),
    description("Solr", &["solr"], &[], None, legacy!("solr", "solr")),
    description("SML", &["sml"], &["sml", "sig", "fun", "smackspec"], None, legacy!("mllike", "sml")),
    description("SPARQL", &["sparul", "sparql"], &["rq", "sparql"], None, legacy!("sparql", "sparql")),
    description("Spreadsheet", &["excel", "formula", "spreadsheet"], &[], None, legacy!("spreadsheet", "spreadsheet")),
    description("Squirrel", &["squirrel"], &["nut"], None, legacy!("clike", "squirrel")),
    description("Stylus", &["stylus"], &["styl"], None, legacy!("stylus", "stylus")),
    description("Swift", &["swift"], &["swift"], None, legacy!("swift", "swift")),
    description("sTeX", &["stex"], &[], None, legacy!("stex", "stex")),
    description("LaTeX", &["tex", "latex"], &["text", "ltx", "tex"], None, legacy!("stex", "stex")),
    description("SystemVerilog", &["systemverilog"], &["v", "sv", "svh"], None, legacy!("verilog", "verilog")),
    description("Tcl", &["tcl"], &["tcl"], None, legacy!("tcl", "tcl")),
    description("Textile", &["textile"], &["textile"], None, legacy!("textile", "textile")),
    description("TiddlyWiki", &["tiddlywiki"], &[], None, legacy!("tiddlywiki", "tiddlyWiki")),
    description("Tiki wiki", &["tiki wiki"], &[], None, legacy!("tiki", "tiki")),
    description("TOML", &["toml"], &["toml"], None, legacy!("toml", "toml")),
    description("Troff", &["troff"], &["1", "2", "3", "4", "5", "6", "7", "8", "9"], None, legacy!("troff", "troff")),
    description("TTCN", &["ttcn"], &["ttcn", "ttcn3", "ttcnpp"], None, legacy!("ttcn", "ttcn")),
    description("TTCN_CFG", &["ttcn_cfg"], &["cfg"], None, legacy!("ttcn-cfg", "ttcnCfg")),
    description("Turtle", &["turtle"], &["ttl"], None, legacy!("turtle", "turtle")),
    description("Web IDL", &["web idl"], &["webidl"], None, legacy!("webidl", "webIDL")),
    description("VB.NET", &["vb.net"], &["vb"], None, legacy!("vb", "vb")),
    description("VBScript", &["vbscript"], &["vbs"], None, legacy!("vbscript", "vbScript")),
    description("Velocity", &["velocity"], &["vtl"], None, legacy!("velocity", "velocity")),
    description("Verilog", &["verilog"], &["v"], None, legacy!("verilog", "verilog")),
    description("VHDL", &["vhdl"], &["vhd", "vhdl"], None, legacy!("vhdl", "vhdl")),
    description("XQuery", &["xquery"], &["xy", "xquery", "xq", "xqm", "xqy"], None, legacy!("xquery", "xQuery")),
    description("Yacas", &["yacas"], &["ys"], None, legacy!("yacas", "yacas")),
    description("Z80", &["z80"], &["z80"], None, legacy!("z80", "z80")),
    description("MscGen", &["mscgen"], &["mscgen", "mscin", "msc"], None, legacy!("mscgen", "mscgen")),
    description("Xù", &["xù"], &["xu"], None, legacy!("mscgen", "xu")),
    description("MsGenny", &["msgenny"], &["msgenny"], None, legacy!("mscgen", "msgenny")),
    description("Vue", &["vue"], &["vue"], None, package!("vue", "vue")),
    description("Angular Template", &["angular template"], &[], None, package!("angular", "angular")),
];
