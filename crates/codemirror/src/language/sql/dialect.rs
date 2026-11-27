use std::{collections::HashMap, sync::OnceLock};

use super::{SqlLanguage, SqlNodeKind};

mod specs;

/// The eight dialects exported by `@codemirror/lang-sql`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SqlDialect {
    #[default]
    StandardSql,
    PostgreSql,
    MySql,
    MariaSql,
    MsSql,
    Sqlite,
    Cassandra,
    PlSql,
}

impl SqlDialect {
    /// Resolve an exact upstream export name.
    pub fn from_export(name: &str) -> Option<Self> {
        match name {
            "StandardSQL" => Some(Self::StandardSql),
            "PostgreSQL" => Some(Self::PostgreSql),
            "MySQL" => Some(Self::MySql),
            "MariaSQL" => Some(Self::MariaSql),
            "MSSQL" => Some(Self::MsSql),
            "SQLite" => Some(Self::Sqlite),
            "Cassandra" => Some(Self::Cassandra),
            "PLSQL" => Some(Self::PlSql),
            _ => None,
        }
    }

    /// The dialect's configuration, including defaults for omitted options.
    pub const fn spec(self) -> SqlDialectSpec<'static> {
        match self {
            Self::StandardSql => SqlDialectSpec::DEFAULT,
            Self::PostgreSql => specs::POSTGRESQL,
            Self::MySql => specs::MYSQL,
            Self::MariaSql => specs::MARIASQL,
            Self::MsSql => specs::MSSQL,
            Self::Sqlite => specs::SQLITE,
            Self::Cassandra => specs::CASSANDRA,
            Self::PlSql => specs::PLSQL,
        }
    }

    pub fn language(self) -> &'static SqlLanguage {
        static BUILT_INS: [OnceLock<SqlLanguage>; 8] = [const { OnceLock::new() }; 8];
        BUILT_INS[self as usize].get_or_init(|| SqlLanguage::define(self.spec()))
    }
}

/// Native counterpart of `SQLDialectSpec`. Word lists use space separators.
/// If `keywords` is empty, the standard word lists replace all three lists.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SqlDialectSpec<'a> {
    pub keywords: &'a str,
    pub builtin: &'a str,
    pub types: &'a str,
    pub backslash_escapes: bool,
    pub hash_comments: bool,
    pub slash_comments: bool,
    pub space_after_dashes: bool,
    pub double_dollar_quoted_strings: bool,
    pub double_quoted_strings: bool,
    pub char_set_casts: bool,
    pub plsql_quoting_mechanism: bool,
    pub operator_chars: &'a str,
    pub special_var: &'a str,
    pub identifier_quotes: &'a str,
    /// Schema completion metadata. It does not affect tokenization.
    pub case_insensitive_identifiers: bool,
    pub unquoted_bit_literals: bool,
    pub treat_bits_as_bytes: bool,
}

impl SqlDialectSpec<'_> {
    pub const DEFAULT: Self = Self {
        keywords: "",
        builtin: "",
        types: "",
        backslash_escapes: false,
        hash_comments: false,
        slash_comments: false,
        space_after_dashes: false,
        double_dollar_quoted_strings: false,
        double_quoted_strings: false,
        char_set_casts: false,
        plsql_quoting_mechanism: false,
        operator_chars: "*+-%<>!=&|~^/",
        special_var: "?",
        identifier_quotes: "\"",
        case_insensitive_identifiers: false,
        unquoted_bit_literals: false,
        treat_bits_as_bytes: false,
    };
}

impl Default for SqlDialectSpec<'_> {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// A word in the dialect's keyword completion table.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SqlWord {
    pub label: String,
    /// Its word-table class before the tokenizer applies lexical rules.
    pub kind: SqlNodeKind,
}

impl SqlWord {
    /// The upstream completion type, which is distinct from the token kind.
    pub const fn completion_type(&self) -> &'static str {
        match self.kind {
            SqlNodeKind::Type => "type",
            SqlNodeKind::Keyword => "keyword",
            _ => "variable",
        }
    }
}

#[derive(Debug)]
pub(super) struct LexConfig {
    pub(super) words: Vec<SqlWord>,
    word_indices: HashMap<String, usize>,
    pub(super) backslash_escapes: bool,
    pub(super) hash_comments: bool,
    pub(super) slash_comments: bool,
    pub(super) space_after_dashes: bool,
    pub(super) double_dollar_quoted_strings: bool,
    pub(super) double_quoted_strings: bool,
    pub(super) char_set_casts: bool,
    pub(super) plsql_quoting_mechanism: bool,
    pub(super) operator_chars: Vec<u16>,
    pub(super) special_var: Vec<u16>,
    pub(super) identifier_quotes: Vec<u16>,
    pub(super) unquoted_bit_literals: bool,
    pub(super) treat_bits_as_bytes: bool,
}

impl LexConfig {
    pub(super) fn new(spec: SqlDialectSpec<'_>) -> Self {
        use SqlNodeKind::{Bool, Builtin, Keyword, Null, Type};
        let mut words: Vec<SqlWord> = Vec::new();
        let mut word_indices = HashMap::<String, usize>::new();
        let (keywords, types, builtin) = if spec.keywords.is_empty() {
            (specs::STANDARD_KEYWORDS, specs::STANDARD_TYPES, "")
        } else {
            (spec.keywords, spec.types, spec.builtin)
        };
        for (list, kind) in [
            ("false true", Bool),
            ("unknown null", Null),
            (keywords, Keyword),
            (types, Type),
            (builtin, Builtin),
        ] {
            for label in list.split(' ').filter(|label| !label.is_empty()) {
                let index = *word_indices.entry(label.to_owned()).or_insert(words.len());
                if index == words.len() {
                    words.push(SqlWord {
                        label: label.to_owned(),
                        kind,
                    });
                } else {
                    words[index].kind = kind;
                }
            }
        }
        // JS Object.keys lists array-index properties first in numeric order.
        let array_index = |label: &str| {
            label
                .parse::<u32>()
                .ok()
                .filter(|&index| index < u32::MAX && index.to_string() == label)
        };
        if words.iter().any(|word| array_index(&word.label).is_some()) {
            words.sort_by_key(|word| array_index(&word.label).unwrap_or(u32::MAX));
            for (index, word) in words.iter().enumerate() {
                *word_indices.get_mut(&word.label).unwrap() = index;
            }
        }
        Self {
            words,
            word_indices,
            backslash_escapes: spec.backslash_escapes,
            hash_comments: spec.hash_comments,
            slash_comments: spec.slash_comments,
            space_after_dashes: spec.space_after_dashes,
            double_dollar_quoted_strings: spec.double_dollar_quoted_strings,
            double_quoted_strings: spec.double_quoted_strings,
            char_set_casts: spec.char_set_casts,
            plsql_quoting_mechanism: spec.plsql_quoting_mechanism,
            operator_chars: spec.operator_chars.encode_utf16().collect(),
            special_var: spec.special_var.encode_utf16().collect(),
            identifier_quotes: spec.identifier_quotes.encode_utf16().collect(),
            unquoted_bit_literals: spec.unquoted_bit_literals,
            treat_bits_as_bytes: spec.treat_bits_as_bytes,
        }
    }

    pub(super) fn word_kind(&self, lowercase: &str) -> SqlNodeKind {
        self.word_indices
            .get(lowercase)
            .map_or(SqlNodeKind::Identifier, |&i| self.words[i].kind)
    }
}
