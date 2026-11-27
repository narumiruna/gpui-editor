use std::ops::Range;

use crate::{Assoc, autocomplete::brackets::CloseBracketSyntax};

use super::PythonDocument;

impl PythonDocument {
    /// Adapt the parsed nodes to quote input in a host document. `origin` is
    /// the byte offset of the Python body in that document.
    pub fn quote_syntax_in(&self, origin: usize) -> impl CloseBracketSyntax + '_ {
        QuoteSyntax { doc: self, origin }
    }
}

struct QuoteSyntax<'a> {
    doc: &'a PythonDocument,
    origin: usize,
}

impl CloseBracketSyntax for QuoteSyntax<'_> {
    fn resolve(&self, pos: usize, side: Option<Assoc>) -> Option<usize> {
        let at = pos.checked_sub(self.origin)?;
        self.doc.resolve_at(at, side).map(|node| node.id)
    }

    fn range(&self, node: usize) -> Range<usize> {
        let range = &self.doc.nodes[node].range;
        self.origin + range.start..self.origin + range.end
    }

    fn parent(&self, node: usize) -> Option<usize> {
        self.doc.nodes[node].parent
    }

    fn first_child(&self, node: usize) -> Option<usize> {
        self.doc.nodes[node].children.first().copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Text;

    #[test]
    fn python_quote_syntax_resolves_byte_boundaries_and_sides() {
        let source = "x = 0\r\n\"😀\"\"b\"";
        let doc = PythonDocument::parse(Text::new(source));
        let origin = "字\r\n".len();
        let syntax = doc.quote_syntax_in(origin);
        let start = origin + "x = 0\r\n".len();
        let edge = start + "\"😀\"".len();
        let end = origin + source.len();
        let range = |at, side| syntax.range(syntax.resolve(at, side).unwrap());

        assert!(!doc.is_recovered());
        assert_eq!(range(edge, Some(Assoc::Before)), start..edge);
        assert_eq!(range(edge, None), start..end);
        assert_eq!(range(edge, Some(Assoc::After)), edge..end);
        assert_eq!(range(edge + 1, None), edge..end);
        assert_eq!(syntax.resolve(origin - 1, None), None);
        assert_eq!(syntax.resolve(start + 2, None), None);
        assert_eq!(syntax.resolve(end + 1, None), None);
    }

    #[test]
    fn python_quote_syntax_does_not_expose_unparsed_text() {
        let doc = PythonDocument::parse(Text::new("\"ok\"\r\nf\"{ }\""));
        let syntax = doc.quote_syntax_in(0);
        assert!(doc.is_recovered());
        let string = syntax.resolve(1, None).unwrap();
        assert_eq!(syntax.range(string), 0..4);
        assert_eq!(syntax.resolve(8, None), None);
    }
}
