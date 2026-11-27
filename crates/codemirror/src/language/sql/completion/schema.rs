use std::{collections::HashMap, sync::Arc};

use crate::autocomplete::session::{CompletionItem, SourceResult};

use super::{
    super::{SqlDialectSpec, SqlDocument},
    context::Context,
    object_key_order, word,
};

/// One column completion or an explicit option with its own metadata.
#[derive(Clone, Debug)]
pub enum SqlSchemaOption {
    Name(String),
    Item(Box<CompletionItem>),
}

impl From<&str> for SqlSchemaOption {
    fn from(name: &str) -> Self {
        Self::Name(name.to_owned())
    }
}

impl From<CompletionItem> for SqlSchemaOption {
    fn from(item: CompletionItem) -> Self {
        Self::Item(Box::new(item))
    }
}

/// The contents of one schema level. `Namespaces` entries retain their order
/// except for array-index keys, which come first in numeric order. In a key,
/// `.` separates levels and `\.` represents a literal dot.
#[derive(Clone, Debug)]
pub enum SqlNamespace {
    Completions(Vec<SqlSchemaOption>),
    Namespaces(Vec<(String, SqlNamespace)>),
    WithSelf {
        completion: Box<CompletionItem>,
        children: Box<SqlNamespace>,
    },
}

/// Schema source inputs. The dialect controls generated identifier quoting;
/// the document supplies the actual syntax tree. Use the same dialect for both.
#[derive(Clone, Copy, Debug, Default)]
pub struct SqlSchemaConfig<'a> {
    /// Use identifier delimiters that fit in one UTF-16 code unit.
    pub dialect: SqlDialectSpec<'a>,
    /// Enable schema completion and supply the root level.
    pub schema: Option<&'a SqlNamespace>,
    /// Make the named schema's options available at the root.
    pub default_schema: Option<&'a str>,
    /// Make the named table's options available at the root.
    pub default_table: Option<&'a str>,
    /// Extra options for tables, supported for compatibility with CodeMirror.
    pub tables: &'a [CompletionItem],
    /// Extra options for schemas, supported for compatibility with CodeMirror.
    pub schemas: &'a [CompletionItem],
}

/// A compiled schema completion source. It does not borrow the input schema.
#[derive(Clone, Debug, Default)]
pub struct SqlSchemaCompletionSource {
    top: Option<Arc<Level<Options>>>,
    default_schema: Option<String>,
    default_table: Option<String>,
}

type Options = Arc<[CompletionItem]>;

/// Build a schema source. Without a schema, it never returns a result.
pub fn schema_completion_source(config: SqlSchemaConfig<'_>) -> SqlSchemaCompletionSource {
    let default_schema = config.default_schema.filter(|name| !name.is_empty());
    let default_table = config.default_table.filter(|name| !name.is_empty());
    let top = config.schema.map(|schema| {
        let names = Names {
            quote: config
                .dialect
                .identifier_quotes
                .chars()
                .next()
                .unwrap_or('"'),
            ignore_case: config.dialect.case_insensitive_identifiers,
        };
        let mut level = Level::default();
        if let Some(name) = default_schema {
            level.child(name, names);
        }
        level.add_namespace(schema, names);
        if let Some(name) = default_schema {
            level
                .child(name, names)
                .extend(config.tables.iter().cloned());
        } else {
            level.extend(config.tables.iter().cloned());
        }
        level.extend(config.schemas.iter().cloned());
        if let Some(name) = default_schema {
            level.extend(level.children[name].list.clone());
        }
        if let Some(name) = default_table {
            let scope = if let Some(schema) = default_schema {
                level.child(schema, names)
            } else {
                &mut level
            };
            let columns = scope.child(name, names).list.clone();
            level.extend(columns);
        }
        Arc::new(level.freeze())
    });
    SqlSchemaCompletionSource {
        top,
        default_schema: default_schema.map(str::to_owned),
        default_table: default_table.map(str::to_owned),
    }
}

impl SqlSchemaCompletionSource {
    pub(super) fn is_enabled(&self) -> bool {
        self.top.is_some()
    }

    /// Return unfiltered options and their replacement range at a valid UTF-8
    /// byte position. Invalid syntax uses the native parser's recovery tree.
    pub fn complete(
        &self,
        doc: &SqlDocument,
        position: usize,
        explicit: bool,
    ) -> Option<SourceResult> {
        let top = self.top.as_deref()?;
        let context = Context::at(doc, position)?;
        if context.empty && !explicit {
            return None;
        }
        let mut parents = context.parents.as_slice();
        if let [parent] = parents
            && let Some((_, path)) = context.aliases.iter().find(|(name, _)| name == parent)
        {
            parents = path;
        }
        let default_schema = self.default_schema.as_ref().map(|name| &top.children[name]);
        let default_table = default_schema
            .zip(self.default_table.as_ref())
            .map(|(scope, name)| &scope.children[name]);
        let mut level = top;
        for name in parents {
            loop {
                if let Some(child) = level.children.get(*name) {
                    level = child;
                    break;
                }
                level = if std::ptr::eq(level, top) {
                    default_schema?
                } else if default_schema.is_some_and(|scope| std::ptr::eq(level, scope)) {
                    default_table?
                } else {
                    return None;
                };
            }
        }
        let aliases = if std::ptr::eq(level, top) {
            context.aliases.as_slice()
        } else {
            &[]
        };
        let with_aliases = || {
            level
                .list
                .iter()
                .cloned()
                .chain(aliases.iter().map(|(name, _)| named(name, "constant")))
        };
        if let Some(open) = context.quoted {
            let close = closing_quote(open);
            let options: Options = with_aliases()
                .map(|mut item| {
                    if !item.label.starts_with(open) {
                        item.label = format!("{open}{}{close}", item.label);
                    }
                    item.insert = None;
                    item
                })
                .collect();
            let mut result = SourceResult::new(context.from, options)
                .with_valid_for(|text, _, _| quoted_span(text));
            if doc.text().as_str()[position..].starts_with(close) {
                result.to = Some(position + close.len_utf8());
            }
            Some(result)
        } else {
            let options = if aliases.is_empty() {
                level.list.clone()
            } else {
                with_aliases().collect()
            };
            Some(
                SourceResult::new(context.from, options)
                    .with_valid_for(|text, _, _| text.encode_utf16().all(word)),
            )
        }
    }
}

fn quoted_span(text: &str) -> bool {
    let text = text.strip_prefix(['`', '\'', '"', '[']).unwrap_or(text);
    let text = text.strip_suffix(['`', '\'', '"', ']']).unwrap_or(text);
    text.encode_utf16().all(word)
}

fn closing_quote(open: char) -> char {
    if open == '[' { ']' } else { open }
}

fn named(label: &str, kind: &str) -> CompletionItem {
    let mut item = CompletionItem::new(label);
    item.kind = Some(kind.to_owned());
    item
}

#[derive(Clone, Copy)]
struct Names {
    quote: char,
    ignore_case: bool,
}

impl Names {
    fn option(self, label: &str, kind: &str) -> CompletionItem {
        let mut item = named(label, kind);
        let letter = |byte: u8| {
            byte == b'_'
                || byte.is_ascii_lowercase()
                || self.ignore_case && byte.is_ascii_uppercase()
        };
        let mut bytes = label.bytes();
        if !bytes.next().is_some_and(letter)
            || !bytes.all(|byte| letter(byte) || byte.is_ascii_digit())
        {
            item.insert = Some(format!(
                "{}{label}{}",
                self.quote,
                closing_quote(self.quote)
            ));
        }
        item
    }
}

#[derive(Clone, Debug, Default)]
struct Level<T> {
    list: T,
    children: HashMap<String, Level<T>>,
}

impl Level<Vec<CompletionItem>> {
    fn child(&mut self, name: &str, names: Names) -> &mut Self {
        self.children.entry(name.to_owned()).or_insert_with(|| {
            if !name.is_empty() && !self.list.iter().any(|item| item.label == name) {
                self.list.push(names.option(name, "type"));
            }
            Self::default()
        })
    }

    fn extend(&mut self, options: impl IntoIterator<Item = CompletionItem>) {
        for option in options {
            if let Some(existing) = self.list.iter_mut().find(|item| item.label == option.label) {
                *existing = option;
            } else {
                self.list.push(option);
            }
        }
    }

    fn add_namespace(&mut self, namespace: &SqlNamespace, names: Names) {
        match namespace {
            SqlNamespace::Completions(options) => {
                self.extend(options.iter().map(|option| match option {
                    SqlSchemaOption::Name(name) => names.option(name, "property"),
                    SqlSchemaOption::Item(item) => item.as_ref().clone(),
                }))
            }
            SqlNamespace::Namespaces(entries) => self.add_namespaces(entries, names),
            SqlNamespace::WithSelf { children, .. } => self.add_namespace(children, names),
        }
    }

    fn add_namespaces(&mut self, entries: &[(String, SqlNamespace)], names: Names) {
        let mut keys: Vec<_> = entries.iter().map(|(name, _)| name.as_str()).collect();
        keys.sort_by_key(|&key| object_key_order(key));
        let mut values: HashMap<_, _> = entries
            .iter()
            .map(|(key, value)| (key.as_str(), value))
            .collect();
        for name in keys {
            // A repeated object key keeps its position but uses its last value.
            let Some(mut children) = values.remove(name) else {
                continue;
            };
            let mut item = None;
            if let SqlNamespace::WithSelf {
                completion,
                children: inner,
            } = children
            {
                item = Some(completion);
                children = inner;
            }
            let mut level = &mut *self;
            let parts = namespace_path(name);
            for (index, part) in parts.iter().enumerate() {
                if index == parts.len() - 1
                    && let Some(item) = item
                {
                    level.extend([item.as_ref().clone()]);
                }
                level = level.child(part, names);
            }
            level.add_namespace(children, names);
        }
    }

    fn freeze(self) -> Level<Options> {
        Level {
            list: self.list.into(),
            children: self
                .children
                .into_iter()
                .map(|(key, level)| (key, level.freeze()))
                .collect(),
        }
    }
}

fn namespace_path(name: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut part = String::new();
    let mut chars = name.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '.' | '\0' => result.push(std::mem::take(&mut part)),
            '\\' if chars.next_if_eq(&'.').is_some() => part.push('.'),
            _ => part.push(ch),
        }
    }
    result.push(part);
    result
}
