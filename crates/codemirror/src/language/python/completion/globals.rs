use std::sync::{Arc, LazyLock};

use crate::autocomplete::{Snippet, session::CompletionItem};

use super::item;

/// A Python keyword completion with a native snippet template.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PythonSnippet {
    pub label: &'static str,
    pub detail: &'static str,
    pub template: &'static str,
}

impl PythonSnippet {
    /// Parse the template for insertion with the native snippet engine.
    pub fn snippet(self) -> Snippet {
        Snippet::parse(self.template)
    }
}

/// The default Python snippet catalog, in upstream order.
pub static SNIPPETS: &[PythonSnippet] = &[
    PythonSnippet {
        label: "def",
        detail: "function",
        template: "def ${name}(${params}):\n\t${}",
    },
    PythonSnippet {
        label: "for",
        detail: "loop",
        template: "for ${name} in ${collection}:\n\t${}",
    },
    PythonSnippet {
        label: "while",
        detail: "loop",
        template: "while ${}:\n\t${}",
    },
    PythonSnippet {
        label: "try",
        detail: "/ except block",
        template: "try:\n\t${}\nexcept ${error}:\n\t${}",
    },
    PythonSnippet {
        label: "if",
        detail: "block",
        template: "if ${}:\n\t\n",
    },
    PythonSnippet {
        label: "if",
        detail: "/ else block",
        template: "if ${}:\n\t${}\nelse:\n\t${}",
    },
    PythonSnippet {
        label: "class",
        detail: "definition",
        template: "class ${name}:\n\tdef __init__(self, ${params}):\n\t\t\t${}",
    },
    PythonSnippet {
        label: "import",
        detail: "statement",
        template: "import ${module}",
    },
    PythonSnippet {
        label: "from",
        detail: "import",
        template: "from ${module} import ${names}",
    },
];

/// Find the native snippet action for an item from the Python global source.
/// The detail distinguishes the two `if` variants. Built-ins return `None`.
pub fn python_completion_snippet(item: &CompletionItem) -> Option<&'static PythonSnippet> {
    if item.kind.as_deref() != Some("keyword") {
        return None;
    }
    let detail = item.detail.as_deref()?;
    SNIPPETS
        .iter()
        .find(|snippet| snippet.label == item.label && snippet.detail == detail)
}

const CONSTANTS: &str = "__annotations__ __builtins__ __debug__ __doc__ __import__ __name__ \
    __loader__ __package__ __spec__ False None True";

const TYPES: &str = "ArithmeticError AssertionError AttributeError BaseException BlockingIOError \
    BrokenPipeError BufferError BytesWarning ChildProcessError ConnectionAbortedError \
    ConnectionError ConnectionRefusedError ConnectionResetError DeprecationWarning \
    EOFError Ellipsis EncodingWarning EnvironmentError Exception FileExistsError \
    FileNotFoundError FloatingPointError FutureWarning GeneratorExit IOError \
    ImportError ImportWarning IndentationError IndexError InterruptedError \
    IsADirectoryError KeyError KeyboardInterrupt LookupError MemoryError \
    ModuleNotFoundError NameError NotADirectoryError NotImplemented NotImplementedError \
    OSError OverflowError PendingDeprecationWarning PermissionError ProcessLookupError \
    RecursionError ReferenceError ResourceWarning RuntimeError RuntimeWarning \
    StopAsyncIteration StopIteration SyntaxError SyntaxWarning SystemError \
    SystemExit TabError TimeoutError TypeError UnboundLocalError UnicodeDecodeError \
    UnicodeEncodeError UnicodeError UnicodeTranslateError UnicodeWarning UserWarning \
    ValueError Warning ZeroDivisionError";

const CLASSES: &str = "bool bytearray bytes classmethod complex float frozenset int list \
    map memoryview object range set staticmethod str super tuple type";

const FUNCTIONS: &str = "abs aiter all anext any ascii bin breakpoint callable chr \
    compile delattr dict dir divmod enumerate eval exec exit filter \
    format getattr globals hasattr hash help hex id input isinstance \
    issubclass iter len license locals max min next oct open \
    ord pow print property quit repr reversed round setattr slice \
    sorted sum vars zip";

pub(super) static OPTIONS: LazyLock<Arc<[CompletionItem]>> = LazyLock::new(|| {
    [
        ("constant", CONSTANTS),
        ("type", TYPES),
        ("class", CLASSES),
        ("function", FUNCTIONS),
    ]
    .into_iter()
    .flat_map(|(kind, entries)| {
        entries
            .split_ascii_whitespace()
            .map(move |label| item(label, kind))
    })
    .chain(SNIPPETS.iter().map(|snippet| {
        let mut option = item(snippet.label, "keyword");
        option.detail = Some(snippet.detail.to_owned());
        option
    }))
    .collect()
});
