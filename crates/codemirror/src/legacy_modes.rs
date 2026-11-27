//! Native stream modes from `@codemirror/legacy-modes` 6.5.5.
//! See `docs/licenses/codemirror-legacy-modes.txt` for the upstream MIT notice.
//!
//! This module implements 58 of the 144 concrete mode exports and includes
//! the CSS keyword table. It does not implement the seven factories.
//! Token positions use UTF-8 byte offsets and do not split scalars. The TOML
//! scanner advances through line separators inside strings instead of stalling.

use std::{borrow::Cow, sync::LazyLock};

use crate::{
    language::{LanguageData, StreamLanguage},
    stream_parser::{StreamParser, StringStream},
};

mod ascii_armor;
pub mod asn1;
mod assembly_modes;
mod brainfuck;
mod build_modes;
mod classic_modes;
mod clike;
mod compiled_modes;
mod config_modes;
mod data_modes;
mod diff;
mod functional_modes;
mod general_modes;
mod grammar_modes;
mod hdl_modes;
mod historical_modes;
mod http;
mod lisp_modes;
mod markup_modes;
mod modern_streams;
mod numeric_modes;
mod presentation_modes;
mod properties;
mod protobuf;
mod query_modes;
mod science_modes;
mod script_modes;
pub mod simple_mode;
mod sql_modes;
mod style_modes;
mod template_streams;
mod terminal_modes;
mod toml;
mod typeset_modes;
mod vendor_modes;
mod web_modes;

pub use clike::{
    LegacyClike, LegacyClikeConfig, LegacyClikeHook, LegacyClikeHookResult, LegacyClikeState,
    LegacyClikeTokenHook, LegacyClikeWords, clike,
};
pub use script_modes::python_factory::{
    LegacyPython, LegacyPythonConfig, LegacyPythonState, mk_python,
};
pub use sql_modes::{
    LegacySql, LegacySqlConfig, LegacySqlHook, LegacySqlHookResult, LegacySqlState,
    LegacySqlSupport, sql,
};
pub use style_modes::{
    CSS_KEYWORDS, CssKeywords, LegacyCss, LegacyCssConfig, LegacyCssHookToken,
    LegacyCssKeywordOverrides, LegacyCssState, LegacyCssTokenHook, LegacyCssTokenKind, mk_css,
};
pub use web_modes::xml_factory::{LegacyXml, LegacyXmlConfig, LegacyXmlState, mk_xml};

/// A bundled mode. Use it directly with a tokenizer, or use [`Self::language`]
/// to get document highlights and language data.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LegacyMode {
    AsciiArmor,
    Brainfuck,
    Diff,
    Http,
    Properties,
    Protobuf,
    Toml,
    C,
    Cpp,
    Java,
    CSharp,
    Scala,
    Kotlin,
    Shader,
    NesC,
    ObjectiveC,
    ObjectiveCpp,
    Squirrel,
    Ceylon,
    Dart,
    CMake,
    Dockerfile,
    Shell,
    R,
    Julia,
    Lua,
    Perl,
    JavaScript,
    Json,
    JsonLd,
    TypeScript,
    Xml,
    Html,
    Haskell,
    OCaml,
    FSharp,
    Sml,
    Erlang,
    Python,
    Cython,
    Ruby,
    Groovy,
    PowerShell,
    Yaml,
    Turtle,
    Ebnf,
    Css,
    Scss,
    Less,
    Gss,
    Apl,
    Fortran,
    Cobol,
    Pascal,
    Smalltalk,
    Clojure,
    CommonLisp,
    Scheme,
    Elm,
    Swift,
    D,
    Crystal,
    Nginx,
    Mbox,
    Asterisk,
    RpmChanges,
    RpmSpec,
    Dtd,
    NTriples,
    Sparql,
    Mirc,
    Tcl,
    Vb,
    Verilog,
    Tlv,
    Vhdl,
    Go,
    Rust,
    Jinja2,
    StandardSql,
    MsSql,
    MySql,
    MariaDb,
    Sqlite,
    Cassandra,
    PlSql,
    Hive,
    PgSql,
    Gql,
    GpSql,
    SparkSql,
    Esper,
    Gherkin,
    Troff,
    Textile,
    TiddlyWiki,
    Tiki,
    Factor,
    Nsis,
    Wast,
    Gas,
    GasArm,
    Z80,
    Ez80,
    Cypher,
    Solr,
    Sieve,
    WebIdl,
    XQuery,
    Mathematica,
    Octave,
    Sas,
    Q,
    Modelica,
    Fcl,
    Forth,
    Ecl,
    Idl,
    Pig,
    Puppet,
    Dylan,
    Eiffel,
    Mumps,
    Oz,
    Yacas,
    Haxe,
    Hxml,
    Spreadsheet,
    Ttcn,
    TtcnCfg,
    Mscgen,
    Msgenny,
    Xu,
    VbScript,
    VbScriptAsp,
    Velocity,
    CoffeeScript,
    LiveScript,
    PegJs,
    Pug,
    Sass,
    Stex,
    StexMath,
    Stylus,
}

enum Family {
    Single,
    Build,
    Classic,
    Clike,
    Compiled,
    Config,
    Data,
    Functional,
    Lisp,
    Science,
    Script,
    Style,
    Web,
    Grammar,
    Hdl,
    Terminal,
    ModernStream,
    Sql,
    Markup,
    Simple,
    Assembly,
    Query,
    Numeric,
    Vendor,
    Historical,
    General,
    Presentation,
    TemplateStreams,
    Typeset,
}

impl LegacyMode {
    pub const ALL: [Self; 144] = [
        Self::AsciiArmor,
        Self::Brainfuck,
        Self::Diff,
        Self::Http,
        Self::Properties,
        Self::Protobuf,
        Self::Toml,
        Self::C,
        Self::Cpp,
        Self::Java,
        Self::CSharp,
        Self::Scala,
        Self::Kotlin,
        Self::Shader,
        Self::NesC,
        Self::ObjectiveC,
        Self::ObjectiveCpp,
        Self::Squirrel,
        Self::Ceylon,
        Self::Dart,
        Self::CMake,
        Self::Dockerfile,
        Self::Shell,
        Self::R,
        Self::Julia,
        Self::Lua,
        Self::Perl,
        Self::JavaScript,
        Self::Json,
        Self::JsonLd,
        Self::TypeScript,
        Self::Xml,
        Self::Html,
        Self::Haskell,
        Self::OCaml,
        Self::FSharp,
        Self::Sml,
        Self::Erlang,
        Self::Python,
        Self::Cython,
        Self::Ruby,
        Self::Groovy,
        Self::PowerShell,
        Self::Yaml,
        Self::Turtle,
        Self::Ebnf,
        Self::Css,
        Self::Scss,
        Self::Less,
        Self::Gss,
        Self::Apl,
        Self::Fortran,
        Self::Cobol,
        Self::Pascal,
        Self::Smalltalk,
        Self::Clojure,
        Self::CommonLisp,
        Self::Scheme,
        Self::Elm,
        Self::Swift,
        Self::D,
        Self::Crystal,
        Self::Nginx,
        Self::Mbox,
        Self::Asterisk,
        Self::RpmChanges,
        Self::RpmSpec,
        Self::Dtd,
        Self::NTriples,
        Self::Sparql,
        Self::Mirc,
        Self::Tcl,
        Self::Vb,
        Self::Verilog,
        Self::Tlv,
        Self::Vhdl,
        Self::Go,
        Self::Rust,
        Self::Jinja2,
        Self::StandardSql,
        Self::MsSql,
        Self::MySql,
        Self::MariaDb,
        Self::Sqlite,
        Self::Cassandra,
        Self::PlSql,
        Self::Hive,
        Self::PgSql,
        Self::Gql,
        Self::GpSql,
        Self::SparkSql,
        Self::Esper,
        Self::Gherkin,
        Self::Troff,
        Self::Textile,
        Self::TiddlyWiki,
        Self::Tiki,
        Self::Factor,
        Self::Nsis,
        Self::Wast,
        Self::Gas,
        Self::GasArm,
        Self::Z80,
        Self::Ez80,
        Self::Cypher,
        Self::Solr,
        Self::Sieve,
        Self::WebIdl,
        Self::XQuery,
        Self::Mathematica,
        Self::Octave,
        Self::Sas,
        Self::Q,
        Self::Modelica,
        Self::Fcl,
        Self::Forth,
        Self::Ecl,
        Self::Idl,
        Self::Pig,
        Self::Puppet,
        Self::Dylan,
        Self::Eiffel,
        Self::Mumps,
        Self::Oz,
        Self::Yacas,
        Self::Haxe,
        Self::Hxml,
        Self::Spreadsheet,
        Self::Ttcn,
        Self::TtcnCfg,
        Self::Mscgen,
        Self::Msgenny,
        Self::Xu,
        Self::VbScript,
        Self::VbScriptAsp,
        Self::Velocity,
        Self::CoffeeScript,
        Self::LiveScript,
        Self::PegJs,
        Self::Pug,
        Self::Sass,
        Self::Stex,
        Self::StexMath,
        Self::Stylus,
    ];

    /// Find a direct mode export by its exact npm subpath and export name.
    pub fn from_export(module: &str, export: &str) -> Option<Self> {
        let name = module.strip_prefix("@codemirror/legacy-modes/mode/")?;
        Self::ALL
            .into_iter()
            .find(|mode| mode.names() == (name, export))
    }

    /// Get the native stream language with the mode's upstream language data.
    pub fn language(self) -> &'static StreamLanguage<Self> {
        static LANGUAGES: LazyLock<[StreamLanguage<LegacyMode>; LegacyMode::ALL.len()]> =
            LazyLock::new(|| {
                LegacyMode::ALL
                    .map(|mode| StreamLanguage::new(mode).with_language_data(mode.data()))
            });
        &LANGUAGES[self as usize]
    }

    fn names(self) -> (&'static str, &'static str) {
        match self {
            Self::AsciiArmor => ("asciiarmor", "asciiArmor"),
            Self::Brainfuck => ("brainfuck", "brainfuck"),
            Self::Diff => ("diff", "diff"),
            Self::Http => ("http", "http"),
            Self::Properties => ("properties", "properties"),
            Self::Protobuf => ("protobuf", "protobuf"),
            Self::Toml => ("toml", "toml"),
            Self::C => ("clike", "c"),
            Self::Cpp => ("clike", "cpp"),
            Self::Java => ("clike", "java"),
            Self::CSharp => ("clike", "csharp"),
            Self::Scala => ("clike", "scala"),
            Self::Kotlin => ("clike", "kotlin"),
            Self::Shader => ("clike", "shader"),
            Self::NesC => ("clike", "nesC"),
            Self::ObjectiveC => ("clike", "objectiveC"),
            Self::ObjectiveCpp => ("clike", "objectiveCpp"),
            Self::Squirrel => ("clike", "squirrel"),
            Self::Ceylon => ("clike", "ceylon"),
            Self::Dart => ("clike", "dart"),
            Self::CMake => ("cmake", "cmake"),
            Self::Dockerfile => ("dockerfile", "dockerFile"),
            Self::Shell => ("shell", "shell"),
            Self::R => ("r", "r"),
            Self::Julia => ("julia", "julia"),
            Self::Lua => ("lua", "lua"),
            Self::Perl => ("perl", "perl"),
            Self::JavaScript => ("javascript", "javascript"),
            Self::Json => ("javascript", "json"),
            Self::JsonLd => ("javascript", "jsonld"),
            Self::TypeScript => ("javascript", "typescript"),
            Self::Xml => ("xml", "xml"),
            Self::Html => ("xml", "html"),
            Self::Haskell => ("haskell", "haskell"),
            Self::OCaml => ("mllike", "oCaml"),
            Self::FSharp => ("mllike", "fSharp"),
            Self::Sml => ("mllike", "sml"),
            Self::Erlang => ("erlang", "erlang"),
            Self::Python => ("python", "python"),
            Self::Cython => ("python", "cython"),
            Self::Ruby => ("ruby", "ruby"),
            Self::Groovy => ("groovy", "groovy"),
            Self::PowerShell => ("powershell", "powerShell"),
            Self::Yaml => ("yaml", "yaml"),
            Self::Turtle => ("turtle", "turtle"),
            Self::Ebnf => ("ebnf", "ebnf"),
            Self::Css => ("css", "css"),
            Self::Scss => ("css", "sCSS"),
            Self::Less => ("css", "less"),
            Self::Gss => ("css", "gss"),
            Self::Apl => ("apl", "apl"),
            Self::Fortran => ("fortran", "fortran"),
            Self::Cobol => ("cobol", "cobol"),
            Self::Pascal => ("pascal", "pascal"),
            Self::Smalltalk => ("smalltalk", "smalltalk"),
            Self::Clojure => ("clojure", "clojure"),
            Self::CommonLisp => ("commonlisp", "commonLisp"),
            Self::Scheme => ("scheme", "scheme"),
            Self::Elm => ("elm", "elm"),
            Self::Swift => ("swift", "swift"),
            Self::D => ("d", "d"),
            Self::Crystal => ("crystal", "crystal"),
            Self::Nginx => ("nginx", "nginx"),
            Self::Mbox => ("mbox", "mbox"),
            Self::Asterisk => ("asterisk", "asterisk"),
            Self::RpmChanges => ("rpm", "rpmChanges"),
            Self::RpmSpec => ("rpm", "rpmSpec"),
            Self::Dtd => ("dtd", "dtd"),
            Self::NTriples => ("ntriples", "ntriples"),
            Self::Sparql => ("sparql", "sparql"),
            Self::Mirc => ("mirc", "mirc"),
            Self::Tcl => ("tcl", "tcl"),
            Self::Vb => ("vb", "vb"),
            Self::Verilog => ("verilog", "verilog"),
            Self::Tlv => ("verilog", "tlv"),
            Self::Vhdl => ("vhdl", "vhdl"),
            Self::Go => ("go", "go"),
            Self::Rust => ("rust", "rust"),
            Self::Jinja2 => ("jinja2", "jinja2"),
            Self::StandardSql => ("sql", "standardSQL"),
            Self::MsSql => ("sql", "msSQL"),
            Self::MySql => ("sql", "mySQL"),
            Self::MariaDb => ("sql", "mariaDB"),
            Self::Sqlite => ("sql", "sqlite"),
            Self::Cassandra => ("sql", "cassandra"),
            Self::PlSql => ("sql", "plSQL"),
            Self::Hive => ("sql", "hive"),
            Self::PgSql => ("sql", "pgSQL"),
            Self::Gql => ("sql", "gql"),
            Self::GpSql => ("sql", "gpSQL"),
            Self::SparkSql => ("sql", "sparkSQL"),
            Self::Esper => ("sql", "esper"),
            Self::Gherkin => ("gherkin", "gherkin"),
            Self::Troff => ("troff", "troff"),
            Self::Textile => ("textile", "textile"),
            Self::TiddlyWiki => ("tiddlywiki", "tiddlyWiki"),
            Self::Tiki => ("tiki", "tiki"),
            Self::Factor => ("factor", "factor"),
            Self::Nsis => ("nsis", "nsis"),
            Self::Wast => ("wast", "wast"),
            Self::Gas => ("gas", "gas"),
            Self::GasArm => ("gas", "gasArm"),
            Self::Z80 => ("z80", "z80"),
            Self::Ez80 => ("z80", "ez80"),
            Self::Cypher => ("cypher", "cypher"),
            Self::Solr => ("solr", "solr"),
            Self::Sieve => ("sieve", "sieve"),
            Self::WebIdl => ("webidl", "webIDL"),
            Self::XQuery => ("xquery", "xQuery"),
            Self::Mathematica => ("mathematica", "mathematica"),
            Self::Octave => ("octave", "octave"),
            Self::Sas => ("sas", "sas"),
            Self::Q => ("q", "q"),
            Self::Modelica => ("modelica", "modelica"),
            Self::Fcl => ("fcl", "fcl"),
            Self::Forth => ("forth", "forth"),
            Self::Ecl => ("ecl", "ecl"),
            Self::Idl => ("idl", "idl"),
            Self::Pig => ("pig", "pig"),
            Self::Puppet => ("puppet", "puppet"),
            Self::Dylan => ("dylan", "dylan"),
            Self::Eiffel => ("eiffel", "eiffel"),
            Self::Mumps => ("mumps", "mumps"),
            Self::Oz => ("oz", "oz"),
            Self::Yacas => ("yacas", "yacas"),
            Self::Haxe => ("haxe", "haxe"),
            Self::Hxml => ("haxe", "hxml"),
            Self::Spreadsheet => ("spreadsheet", "spreadsheet"),
            Self::Ttcn => ("ttcn", "ttcn"),
            Self::TtcnCfg => ("ttcn-cfg", "ttcnCfg"),
            Self::Mscgen => ("mscgen", "mscgen"),
            Self::Msgenny => ("mscgen", "msgenny"),
            Self::Xu => ("mscgen", "xu"),
            Self::VbScript => ("vbscript", "vbScript"),
            Self::VbScriptAsp => ("vbscript", "vbScriptASP"),
            Self::Velocity => ("velocity", "velocity"),
            Self::CoffeeScript => ("coffeescript", "coffeeScript"),
            Self::LiveScript => ("livescript", "liveScript"),
            Self::PegJs => ("pegjs", "pegjs"),
            Self::Pug => ("pug", "pug"),
            Self::Sass => ("sass", "sass"),
            Self::Stex => ("stex", "stex"),
            Self::StexMath => ("stex", "stexMath"),
            Self::Stylus => ("stylus", "stylus"),
        }
    }

    fn family(self) -> Family {
        match self.names().0 {
            "cmake" | "dockerfile" | "shell" => Family::Build,
            "apl" | "fortran" | "cobol" | "pascal" | "smalltalk" => Family::Classic,
            "clike" => Family::Clike,
            "elm" | "swift" | "d" | "crystal" => Family::Compiled,
            "nginx" | "mbox" | "asterisk" | "rpm" => Family::Config,
            "yaml" | "turtle" | "ebnf" => Family::Data,
            "haskell" | "mllike" | "erlang" => Family::Functional,
            "clojure" | "commonlisp" | "scheme" => Family::Lisp,
            "r" | "julia" | "lua" | "perl" => Family::Science,
            "python" | "ruby" | "groovy" | "powershell" => Family::Script,
            "css" => Family::Style,
            "javascript" | "xml" => Family::Web,
            "dtd" | "ntriples" | "sparql" => Family::Grammar,
            "verilog" | "vhdl" => Family::Hdl,
            "mirc" | "tcl" | "vb" => Family::Terminal,
            "go" | "jinja2" | "rust" => Family::ModernStream,
            "sql" => Family::Sql,
            "gas" | "z80" => Family::Assembly,
            "cypher" | "solr" | "sieve" | "webidl" | "xquery" => Family::Query,
            "mathematica" | "octave" | "sas" | "q" | "modelica" => Family::Numeric,
            "fcl" | "forth" | "ecl" | "idl" | "pig" | "puppet" => Family::Vendor,
            "dylan" | "eiffel" | "mumps" | "oz" | "yacas" => Family::Historical,
            "haxe" | "spreadsheet" | "ttcn" | "ttcn-cfg" => Family::General,
            "mscgen" | "vbscript" | "velocity" => Family::Presentation,
            "coffeescript" | "livescript" | "pegjs" | "pug" => Family::TemplateStreams,
            "sass" | "stex" | "stylus" => Family::Typeset,
            "gherkin" | "troff" | "textile" | "tiddlywiki" | "tiki" => Family::Markup,
            "factor" | "nsis" | "wast" => Family::Simple,
            _ => Family::Single,
        }
    }

    fn data(self) -> LanguageData {
        match self.family() {
            Family::Build => build_modes::data(self),
            Family::Classic => classic_modes::data(self),
            Family::Clike => clike::data(self),
            Family::Compiled => compiled_modes::data(self),
            Family::Config => config_modes::data(self),
            Family::Data => data_modes::data(self),
            Family::Functional => functional_modes::data(self),
            Family::Lisp => lisp_modes::data(self),
            Family::Science => science_modes::data(self),
            Family::Script => script_modes::data(self),
            Family::Style => style_modes::data(self),
            Family::Web => web_modes::data(self),
            Family::Grammar => grammar_modes::data(self),
            Family::Hdl => hdl_modes::data(self),
            Family::Terminal => terminal_modes::data(self),
            Family::ModernStream => modern_streams::data(self),
            Family::Sql => sql_modes::data(self),
            Family::Assembly => assembly_modes::data(self),
            Family::Query => query_modes::data(self),
            Family::Numeric => numeric_modes::data(self),
            Family::Vendor => vendor_modes::data(self),
            Family::Historical => historical_modes::data(self),
            Family::General => general_modes::data(self),
            Family::Presentation => presentation_modes::data(self),
            Family::Typeset => typeset_modes::data(self),
            Family::TemplateStreams => template_streams::data(self),
            Family::Markup => LanguageData::default(),
            Family::Simple => simple_mode::data(self),
            Family::Single => {
                let data = LanguageData::default();
                match self {
                    Self::Protobuf => data.with("autocomplete", protobuf::KEYWORDS),
                    Self::Toml => data.with("commentTokens", CommentTokens { line: "#" }),
                    _ => data,
                }
            }
        }
    }
}

/// The `commentTokens` language data for a mode with line comments.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CommentTokens {
    pub line: &'static str,
}

/// A state created by a [`LegacyMode`]. Use it only with that mode.
#[derive(Clone, Debug)]
pub struct LegacyState(State);

#[derive(Clone, Debug)]
enum State {
    AsciiArmor(ascii_armor::State),
    Brainfuck(brainfuck::State),
    Diff,
    Http(http::State),
    Properties(properties::State),
    Protobuf,
    Toml(toml::State),
    Clike(clike::State),
    Build(build_modes::State),
    Science(science_modes::State),
    Web(web_modes::State),
    Functional(functional_modes::State),
    Script(script_modes::State),
    Data(data_modes::State),
    Style(style_modes::State),
    Classic(classic_modes::State),
    Lisp(lisp_modes::State),
    Compiled(compiled_modes::State),
    Config(config_modes::State),
    Grammar(grammar_modes::State),
    Hdl(hdl_modes::State),
    Terminal(terminal_modes::State),
    ModernStream(modern_streams::State),
    Sql(sql_modes::State),
    Assembly(assembly_modes::State),
    Query(query_modes::State),
    Numeric(numeric_modes::State),
    Vendor(vendor_modes::State),
    Historical(historical_modes::State),
    General(general_modes::State),
    Presentation(presentation_modes::State),
    Typeset(typeset_modes::State),
    TemplateStreams(template_streams::State),
    Markup(markup_modes::State),
    Simple(LegacyMode, simple_mode::SimpleModeState),
}

impl StreamParser for LegacyMode {
    type State = LegacyState;

    fn name(&self) -> Option<&str> {
        match self.family() {
            Family::Build => build_modes::name(*self),
            Family::Classic => classic_modes::name(*self),
            Family::Clike => Some(clike::name(*self)),
            Family::Compiled => compiled_modes::name(*self),
            Family::Config => config_modes::name(*self),
            Family::Data => data_modes::name(*self),
            Family::Functional => functional_modes::name(*self),
            Family::Lisp => lisp_modes::name(*self),
            Family::Science => science_modes::name(*self),
            Family::Script => script_modes::name(*self),
            Family::Style => style_modes::name(*self),
            Family::Web => web_modes::name(*self),
            Family::Grammar => grammar_modes::name(*self),
            Family::Hdl => hdl_modes::name(*self),
            Family::Terminal => terminal_modes::name(*self),
            Family::ModernStream => modern_streams::name(*self),
            Family::Sql => sql_modes::name(*self),
            Family::Assembly => assembly_modes::name(*self),
            Family::Query => query_modes::name(*self),
            Family::Numeric => numeric_modes::name(*self),
            Family::Vendor => vendor_modes::name(*self),
            Family::Historical => historical_modes::name(*self),
            Family::General => general_modes::name(*self),
            Family::Presentation => presentation_modes::name(*self),
            Family::Typeset => typeset_modes::name(*self),
            Family::TemplateStreams => template_streams::name(*self),
            Family::Markup => markup_modes::name(*self),
            Family::Simple => simple_mode::bundled(*self).name(),
            Family::Single => Some(self.names().0),
        }
    }

    fn start_state(&self, indent_unit: usize) -> LegacyState {
        LegacyState(match self {
            Self::AsciiArmor => State::AsciiArmor(ascii_armor::State::default()),
            Self::Brainfuck => State::Brainfuck(brainfuck::State::default()),
            Self::Diff => State::Diff,
            Self::Http => State::Http(http::State::default()),
            Self::Properties => State::Properties(properties::State::default()),
            Self::Protobuf => State::Protobuf,
            Self::Toml => State::Toml(toml::State::default()),
            mode => match mode.family() {
                Family::Build => State::Build(build_modes::State::new(*mode)),
                Family::Classic => State::Classic(classic_modes::State::new(*mode)),
                Family::Clike => State::Clike(clike::State::new(*mode, indent_unit)),
                Family::Compiled => State::Compiled(compiled_modes::State::new(*mode, indent_unit)),
                Family::Config => State::Config(config_modes::State::new(*mode)),
                Family::Data => State::Data(data_modes::State::new(*mode)),
                Family::Functional => {
                    State::Functional(functional_modes::State::new(*mode, indent_unit))
                }
                Family::Lisp => State::Lisp(lisp_modes::State::new(*mode, indent_unit)),
                Family::Science => State::Science(science_modes::State::new(*mode, indent_unit)),
                Family::Script => State::Script(script_modes::State::new(*mode, indent_unit)),
                Family::Style => State::Style(style_modes::State::new(*mode)),
                Family::Web => State::Web(web_modes::State::new(*mode, indent_unit)),
                Family::Grammar => State::Grammar(grammar_modes::State::new(*mode)),
                Family::Hdl => State::Hdl(hdl_modes::State::new(*mode, indent_unit)),
                Family::Terminal => State::Terminal(terminal_modes::State::new(*mode)),
                Family::ModernStream => {
                    State::ModernStream(modern_streams::State::new(*mode, indent_unit))
                }
                Family::Sql => State::Sql(sql_modes::State::new(*mode)),
                Family::Assembly => State::Assembly(assembly_modes::State::new(*mode)),
                Family::Query => State::Query(query_modes::State::new(*mode)),
                Family::Numeric => State::Numeric(numeric_modes::State::new(*mode, indent_unit)),
                Family::Vendor => State::Vendor(vendor_modes::State::new(*mode, indent_unit)),
                Family::Historical => State::Historical(historical_modes::State::new(*mode)),
                Family::General => State::General(general_modes::State::new(*mode, indent_unit)),
                Family::Presentation => State::Presentation(presentation_modes::State::new(*mode)),
                Family::Typeset => State::Typeset(typeset_modes::State::new(*mode)),
                Family::TemplateStreams => {
                    State::TemplateStreams(template_streams::State::new(*mode, indent_unit))
                }
                Family::Markup => State::Markup(markup_modes::State::new(*mode)),
                Family::Simple => {
                    State::Simple(*mode, simple_mode::bundled(*mode).start_state(indent_unit))
                }
                Family::Single => unreachable!("all single modes have their own state"),
            },
        })
    }

    fn token(
        &self,
        stream: &mut StringStream<'_>,
        state: &mut LegacyState,
    ) -> Option<Cow<'_, str>> {
        match (self, &mut state.0) {
            (Self::AsciiArmor, State::AsciiArmor(state)) => state.token(stream),
            (Self::Brainfuck, State::Brainfuck(state)) => state.token(stream),
            (Self::Diff, State::Diff) => diff::token(stream),
            (Self::Http, State::Http(state)) => state.token(stream),
            (Self::Properties, State::Properties(state)) => state.token(stream),
            (Self::Protobuf, State::Protobuf) => protobuf::token(stream),
            (Self::Toml, State::Toml(state)) => state.token(stream),
            (mode, State::Clike(state)) if *mode == state.mode() => state.token(stream),
            (mode, State::Build(state)) if *mode == state.mode() => state.token(stream),
            (mode, State::Science(state)) if *mode == state.mode() => state.token(stream),
            (mode, State::Web(state)) if *mode == state.mode() => state.token(stream),
            (mode, State::Functional(state)) if *mode == state.mode() => state.token(stream),
            (mode, State::Script(state)) if *mode == state.mode() => state.token(stream),
            (mode, State::Data(state)) if *mode == state.mode() => state.token(stream),
            (mode, State::Style(state)) if *mode == state.mode() => state.token(stream),
            (mode, State::Classic(state)) if *mode == state.mode() => state.token(stream),
            (mode, State::Lisp(state)) if *mode == state.mode() => state.token(stream),
            (mode, State::Compiled(state)) if *mode == state.mode() => state.token(stream),
            (mode, State::Config(state)) if *mode == state.mode() => state.token(stream),
            (mode, State::Grammar(state)) if *mode == state.mode() => state.token(stream),
            (mode, State::Hdl(state)) if *mode == state.mode() => state.token(stream),
            (mode, State::Terminal(state)) if *mode == state.mode() => state.token(stream),
            (mode, State::ModernStream(state)) if *mode == state.mode() => state.token(stream),
            (mode, State::Sql(state)) if *mode == state.mode() => state.token(stream),
            (mode, State::Assembly(state)) if *mode == state.mode() => state.token(stream),
            (mode, State::Query(state)) if *mode == state.mode() => state.token(stream),
            (mode, State::Numeric(state)) if *mode == state.mode() => state.token(stream),
            (mode, State::Vendor(state)) if *mode == state.mode() => state.token(stream),
            (mode, State::Historical(state)) if *mode == state.mode() => state.token(stream),
            (mode, State::General(state)) if *mode == state.mode() => state.token(stream),
            (mode, State::Presentation(state)) if *mode == state.mode() => state.token(stream),
            (mode, State::Typeset(state)) if *mode == state.mode() => state.token(stream),
            (mode, State::TemplateStreams(state)) if *mode == state.mode() => state.token(stream),
            (mode, State::Markup(state)) if *mode == state.mode() => return state.token(stream),
            (mode, State::Simple(owner, state)) if mode == owner => {
                return simple_mode::bundled(*mode).token(stream, state);
            }
            _ => panic!("stream state belongs to a different legacy mode"),
        }
        .map(Cow::Borrowed)
    }

    fn indent(&self, state: &LegacyState, text_after: &str, indent_unit: usize) -> Option<usize> {
        match &state.0 {
            State::Clike(state) if *self == state.mode() => state.indent(text_after, indent_unit),
            State::Build(state) if *self == state.mode() => state.indent(text_after, indent_unit),
            State::Science(state) if *self == state.mode() => state.indent(text_after, indent_unit),
            State::Web(state) if *self == state.mode() => state.indent(text_after, indent_unit),
            State::Functional(state) if *self == state.mode() => {
                state.indent(text_after, indent_unit)
            }
            State::Script(state) if *self == state.mode() => state.indent(text_after, indent_unit),
            State::Data(state) if *self == state.mode() => state.indent(text_after, indent_unit),
            State::Style(state) if *self == state.mode() => state.indent(text_after, indent_unit),
            State::Classic(state) if *self == state.mode() => state.indent(text_after, indent_unit),
            State::Lisp(state) if *self == state.mode() => state.indent(text_after, indent_unit),
            State::Compiled(state) if *self == state.mode() => {
                state.indent(text_after, indent_unit)
            }
            State::Config(state) if *self == state.mode() => state.indent(text_after, indent_unit),
            State::Grammar(state) if *self == state.mode() => state.indent(text_after, indent_unit),
            State::Hdl(state) if *self == state.mode() => state.indent(text_after, indent_unit),
            State::Terminal(state) if *self == state.mode() => {
                state.indent(text_after, indent_unit)
            }
            State::ModernStream(state) if *self == state.mode() => {
                state.indent(text_after, indent_unit)
            }
            State::Sql(state) if *self == state.mode() => state.indent(text_after, indent_unit),
            State::Query(state) if *self == state.mode() => state.indent(text_after, indent_unit),
            State::Numeric(state) if *self == state.mode() => state.indent(text_after, indent_unit),
            State::Vendor(state) if *self == state.mode() => state.indent(text_after, indent_unit),
            State::Historical(state) if *self == state.mode() => {
                state.indent(text_after, indent_unit)
            }
            State::General(state) if *self == state.mode() => state.indent(text_after, indent_unit),
            State::Presentation(state) if *self == state.mode() => {
                state.indent(text_after, indent_unit)
            }
            State::Typeset(state) if *self == state.mode() => state.indent(text_after, indent_unit),
            State::TemplateStreams(state) if *self == state.mode() => {
                state.indent(text_after, indent_unit)
            }
            State::Markup(state) if *self == state.mode() => state.indent(text_after, indent_unit),
            State::Simple(owner, state) if self == owner => {
                simple_mode::bundled(*self).indent(state, text_after, indent_unit)
            }
            _ => None,
        }
    }

    fn blank_line(&self, state: &mut LegacyState, indent_unit: usize) {
        match (self, &mut state.0) {
            (Self::AsciiArmor, State::AsciiArmor(state)) => state.blank_line(),
            (Self::Http, State::Http(state)) => *state = http::State::Body,
            (mode, State::Classic(state)) if *mode == state.mode() => state.blank_line(indent_unit),
            (mode, State::Config(state)) if *mode == state.mode() => state.blank_line(indent_unit),
            (mode, State::Markup(state)) if *mode == state.mode() => state.blank_line(),
            (mode, State::Typeset(state)) if *mode == state.mode() => state.blank_line(),
            (mode, State::Simple(owner, state)) if mode == owner => {
                simple_mode::bundled(*mode).blank_line(state, indent_unit)
            }
            _ => {}
        }
    }

    fn merge_tokens(&self) -> bool {
        match self.family() {
            Family::Simple => simple_mode::bundled(*self).merge_tokens(),
            _ => true,
        }
    }
}

fn js_space(ch: char) -> bool {
    (ch.is_whitespace() && ch != '\u{85}') || ch == '\u{feff}'
}

fn js_line_terminator(ch: char) -> bool {
    matches!(ch, '\n' | '\r' | '\u{2028}' | '\u{2029}')
}
