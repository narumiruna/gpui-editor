use std::sync::{Arc, LazyLock};

use crate::{EditorState, Extension, Facet};

use super::{SqlSchemaCompletionSource, SqlSchemaConfig, schema_completion_source};
use crate::language::sql::SqlDialect;

/// Register an in-memory schema for native completion in a bundled SQL dialect.
/// The first registration for that dialect in extension precedence order wins.
#[derive(Clone, Debug)]
pub struct NativeSqlSchemaExtension {
    dialect: SqlDialect,
    source: SqlSchemaCompletionSource,
}

impl NativeSqlSchemaExtension {
    /// Compile the schema without borrowing it. The known dialect replaces
    /// `config.dialect` so identifier quoting matches the native parser.
    pub fn new(dialect: SqlDialect, config: SqlSchemaConfig<'_>) -> Self {
        Self {
            dialect,
            source: schema_completion_source(SqlSchemaConfig {
                dialect: dialect.spec(),
                ..config
            }),
        }
    }

    /// A config without a schema installs nothing.
    pub fn extension(self) -> Extension {
        if self.source.is_enabled() {
            facet(self.dialect).of(self.source)
        } else {
            Extension::default()
        }
    }

    /// Read the installed source for this dialect. Keep the returned `Arc`
    /// to check its identity after changes to the editor configuration.
    pub fn source_for(
        state: &EditorState,
        dialect: SqlDialect,
    ) -> Option<Arc<SqlSchemaCompletionSource>> {
        let source = state
            .context()
            .facet(facet(dialect))
            .expect("typed SQL schema facet");
        source.is_enabled().then_some(source)
    }
}

type SchemaFacet = Facet<SqlSchemaCompletionSource, SqlSchemaCompletionSource>;

fn facet(dialect: SqlDialect) -> &'static SchemaFacet {
    static SCHEMAS: [LazyLock<SchemaFacet>; 8] = [const {
        LazyLock::new(|| {
            Facet::new(|inputs: &[&SqlSchemaCompletionSource]| {
                inputs.first().copied().cloned().unwrap_or_default()
            })
        })
    }; 8];
    &SCHEMAS[dialect as usize]
}
