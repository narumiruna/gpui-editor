use std::sync::{Arc, LazyLock};

use crate::autocomplete::{Snippet, session::CompletionItem};

use super::item;

/// A keyword completion with a native snippet template.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct JavascriptSnippet {
    pub label: &'static str,
    pub detail: &'static str,
    pub template: &'static str,
}

impl JavascriptSnippet {
    /// Parse the template for insertion with the native snippet engine.
    pub fn snippet(self) -> Snippet {
        Snippet::parse(self.template)
    }
}

/// The upstream JavaScript snippet catalog, in upstream order.
pub static SNIPPETS: &[JavascriptSnippet] = TYPESCRIPT_SNIPPETS.split_at(11).0;

/// The upstream TypeScript snippet catalog, including all JavaScript snippets.
pub static TYPESCRIPT_SNIPPETS: &[JavascriptSnippet] = &[
    JavascriptSnippet {
        label: "function",
        detail: "definition",
        template: "function ${name}(${params}) {\n\t${}\n}",
    },
    JavascriptSnippet {
        label: "for",
        detail: "loop",
        template: "for (let ${index} = 0; ${index} < ${bound}; ${index}++) {\n\t${}\n}",
    },
    JavascriptSnippet {
        label: "for",
        detail: "of loop",
        template: "for (let ${name} of ${collection}) {\n\t${}\n}",
    },
    JavascriptSnippet {
        label: "do",
        detail: "loop",
        template: "do {\n\t${}\n} while (${})",
    },
    JavascriptSnippet {
        label: "while",
        detail: "loop",
        template: "while (${}) {\n\t${}\n}",
    },
    JavascriptSnippet {
        label: "try",
        detail: "/ catch block",
        template: "try {\n\t${}\n} catch (${error}) {\n\t${}\n}",
    },
    JavascriptSnippet {
        label: "if",
        detail: "block",
        template: "if (${}) {\n\t${}\n}",
    },
    JavascriptSnippet {
        label: "if",
        detail: "/ else block",
        template: "if (${}) {\n\t${}\n} else {\n\t${}\n}",
    },
    JavascriptSnippet {
        label: "class",
        detail: "definition",
        template: "class ${name} {\n\tconstructor(${params}) {\n\t\t${}\n\t}\n}",
    },
    JavascriptSnippet {
        label: "import",
        detail: "named",
        template: "import {${names}} from \"${module}\"\n${}",
    },
    JavascriptSnippet {
        label: "import",
        detail: "default",
        template: "import ${name} from \"${module}\"\n${}",
    },
    JavascriptSnippet {
        label: "interface",
        detail: "definition",
        template: "interface ${name} {\n\t${}\n}",
    },
    JavascriptSnippet {
        label: "type",
        detail: "definition",
        template: "type ${name} = ${type}",
    },
    JavascriptSnippet {
        label: "enum",
        detail: "definition",
        template: "enum ${name} {\n\t${}\n}",
    },
];

/// Find the template for a snippet item from the fixed completion source.
/// Call this only for items from that source. Plain keywords return `None`.
pub fn javascript_completion_snippet(item: &CompletionItem) -> Option<&'static JavascriptSnippet> {
    if item.kind.as_deref() != Some("keyword") {
        return None;
    }
    let detail = item.detail.as_deref()?;
    TYPESCRIPT_SNIPPETS
        .iter()
        .find(|snippet| snippet.label == item.label && snippet.detail == detail)
}

const KEYWORDS: &str = "break case const continue default delete export extends false finally in \
    instanceof let new return static super switch this throw true typeof var yield";
const TYPESCRIPT_KEYWORDS: &str = "declare implements private protected public";

static JS_OPTIONS: LazyLock<Arc<[CompletionItem]>> = LazyLock::new(|| build_options(false));
static TS_OPTIONS: LazyLock<Arc<[CompletionItem]>> = LazyLock::new(|| build_options(true));

pub(super) fn options(typescript: bool) -> Arc<[CompletionItem]> {
    Arc::clone(if typescript { &TS_OPTIONS } else { &JS_OPTIONS })
}

fn build_options(typescript: bool) -> Arc<[CompletionItem]> {
    let snippets = if typescript {
        TYPESCRIPT_SNIPPETS
    } else {
        SNIPPETS
    };
    let extra_keywords = if typescript { TYPESCRIPT_KEYWORDS } else { "" };
    snippets
        .iter()
        .map(|snippet| {
            let mut option = item(snippet.label, "keyword");
            option.detail = Some(snippet.detail.to_owned());
            option
        })
        .chain(
            KEYWORDS
                .split_ascii_whitespace()
                .chain(extra_keywords.split_ascii_whitespace())
                .map(|keyword| item(keyword, "keyword")),
        )
        .collect()
}
