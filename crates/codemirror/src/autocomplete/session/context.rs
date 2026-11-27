use super::{CompletionQuery, QueryMatch};

/// A node in a host-resolved syntax path. Give the nodes from innermost to
/// outermost, resolving to the left of the query position at node boundaries.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CompletionNode<'a> {
    pub name: &'a str,
    pub from: usize,
    /// Whether this node is the root of its language, including a nested one.
    pub is_top: bool,
}

/// The part of a named syntax node before the query position.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct QueryToken<'text, 'syntax> {
    pub node: CompletionNode<'syntax>,
    pub matched: QueryMatch<'text>,
}

impl CompletionQuery {
    /// Find the first named node in the supplied syntax path, and read its text
    /// up to the query position. This lookup can cross a language root.
    pub fn token_before<'a>(
        &self,
        names: &[&str],
        path: &[CompletionNode<'a>],
    ) -> Option<QueryToken<'_, 'a>> {
        let node = *path.iter().find(|node| names.contains(&node.name))?;
        let range = node.from..self.position;
        let text = self.state().text().slice(range.clone())?;
        Some(QueryToken {
            node,
            matched: QueryMatch { range, text },
        })
    }
}

/// Wrap a source so it runs only inside a named node. The caller supplies the
/// syntax path with each query. The lookup includes the first language root
/// but does not inspect its parents.
pub fn if_in<'a, T>(
    names: &'a [&str],
    source: impl Fn(&CompletionQuery) -> Option<T> + 'a,
) -> impl Fn(&CompletionQuery, &[CompletionNode<'_>]) -> Option<T> + 'a {
    move |query, path| {
        if has_node(names, path) {
            source(query)
        } else {
            None
        }
    }
}

/// Wrap a source so it does not run inside any named node. The lookup uses the
/// same language boundary as [`if_in`].
pub fn if_not_in<'a, T>(
    names: &'a [&str],
    source: impl Fn(&CompletionQuery) -> Option<T> + 'a,
) -> impl Fn(&CompletionQuery, &[CompletionNode<'_>]) -> Option<T> + 'a {
    move |query, path| {
        if has_node(names, path) {
            None
        } else {
            source(query)
        }
    }
}

fn has_node(names: &[&str], path: &[CompletionNode<'_>]) -> bool {
    for node in path {
        if names.contains(&node.name) {
            return true;
        }
        if node.is_top {
            break;
        }
    }
    false
}
