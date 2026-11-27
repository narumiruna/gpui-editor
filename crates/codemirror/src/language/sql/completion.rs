//! Native completion sources from `@codemirror/lang-sql` 6.10.0.
//! See `docs/licenses/codemirror-lang-sql.txt`.

mod context;
mod keyword;
mod native;
mod schema;

pub use keyword::{SqlKeywordCompletionSource, keyword_completion_source};
pub use native::NativeSqlSchemaExtension;
pub use schema::{
    SqlNamespace, SqlSchemaCompletionSource, SqlSchemaConfig, SqlSchemaOption,
    schema_completion_source,
};

fn word(unit: u16) -> bool {
    u8::try_from(unit).is_ok_and(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

// Object.keys puts canonical array indices before the other keys.
fn object_key_order(key: &str) -> u32 {
    key.parse::<u32>()
        .ok()
        .filter(|index| index.to_string() == key)
        .unwrap_or(u32::MAX)
}
