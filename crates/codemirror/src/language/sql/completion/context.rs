use super::super::{Bias, SqlDocument, SqlNode, SqlNodeKind::*};

pub(super) struct Context<'a> {
    pub from: usize,
    pub quoted: Option<char>,
    pub parents: Vec<&'a str>,
    pub empty: bool,
    pub aliases: Vec<(&'a str, Vec<&'a str>)>,
}

impl<'a> Context<'a> {
    pub fn at(doc: &'a SqlDocument, position: usize) -> Option<Self> {
        if !doc.text().is_char_boundary(position) {
            return None;
        }
        let node = doc.resolve(position, Bias::Left);
        let aliases = aliases(doc, node);
        let mut context = Self {
            from: position,
            quoted: None,
            parents: Vec::new(),
            empty: false,
            aliases,
        };
        let previous = match node.kind() {
            Identifier | QuotedIdentifier | Keyword => {
                context.from = node.range().start;
                if node.kind() == QuotedIdentifier {
                    context.quoted = node_text(doc, node).chars().next();
                }
                token_before(doc, node)
            }
            Dot => node,
            _ => {
                context.empty = true;
                return Some(context);
            }
        };
        context.parents = parents(doc, previous);
        Some(context)
    }
}

fn token_before<'a>(doc: &'a SqlDocument, node: SqlNode<'a>) -> SqlNode<'a> {
    let mut previous = doc.resolve(node.range().start, Bias::Left);
    while previous.kind().is_comment() {
        previous = doc.resolve(previous.range().start, Bias::Left);
    }
    previous
}

fn node_text<'a>(doc: &'a SqlDocument, node: SqlNode<'a>) -> &'a str {
    doc.text().slice(node.range()).unwrap()
}

fn id_name<'a>(doc: &'a SqlDocument, node: SqlNode<'a>) -> &'a str {
    let text = node_text(doc, node);
    // Upstream removes one opening and one closing quote, even if they
    // differ, but does not unescape the contents or strip multiline quotes.
    if let Some(body) = text
        .strip_prefix(['`', '\'', '"', '['])
        .and_then(|rest| rest.strip_suffix(['`', '\'', '"', ']']))
        && !body.contains(['\r', '\n', '\u{2028}', '\u{2029}'])
    {
        return body;
    }
    text
}

fn plain_id(node: SqlNode<'_>) -> bool {
    matches!(node.kind(), Identifier | QuotedIdentifier)
}

fn parents<'a>(doc: &'a SqlDocument, mut node: SqlNode<'a>) -> Vec<&'a str> {
    let mut path = Vec::new();
    while node.kind() == Dot {
        let name = token_before(doc, node);
        if !plain_id(name) {
            break;
        }
        path.push(id_name(doc, name));
        node = token_before(doc, name);
    }
    path.reverse();
    path
}

fn aliases<'a>(doc: &'a SqlDocument, at: SqlNode<'a>) -> Vec<(&'a str, Vec<&'a str>)> {
    let mut aliases: Vec<(&str, Vec<&str>)> = Vec::new();
    let Some(statement) = at.ancestors().find(|node| node.kind() == Statement) else {
        return aliases;
    };
    let mut children = statement.children().peekable();
    let mut saw_from = false;
    let mut previous = None;
    while let Some(node) = children.next() {
        let keyword = (node.kind() == Keyword).then(|| node_text(doc, node).to_ascii_lowercase());
        let alias = if !saw_from {
            saw_from = keyword.as_deref() == Some("from");
            None
        } else if keyword.as_deref() == Some("as") {
            children.peek().copied().filter(|node| plain_id(*node))
        } else if matches!(
            keyword.as_deref(),
            Some(
                "where"
                    | "group"
                    | "having"
                    | "order"
                    | "union"
                    | "intersect"
                    | "except"
                    | "all"
                    | "distinct"
                    | "limit"
                    | "offset"
                    | "fetch"
                    | "for"
            )
        ) {
            break;
        } else {
            plain_id(node).then_some(node)
        };
        if let Some((id, alias)) = previous.zip(alias) {
            let name = id_name(doc, alias);
            if !name.is_empty() {
                let path = path_for(doc, id);
                if let Some(entry) = aliases.iter_mut().find(|(key, _)| *key == name) {
                    entry.1 = path;
                } else {
                    aliases.push((name, path));
                }
            }
        }
        previous = matches!(
            node.kind(),
            Identifier | QuotedIdentifier | CompositeIdentifier
        )
        .then_some(node);
    }
    aliases.sort_by_key(|(name, _)| super::object_key_order(name));
    aliases
}

fn path_for<'a>(doc: &'a SqlDocument, id: SqlNode<'a>) -> Vec<&'a str> {
    if id.kind() == CompositeIdentifier {
        id.children()
            .filter(|&node| plain_id(node))
            .map(|node| id_name(doc, node))
            .collect()
    } else {
        vec![id_name(doc, id)]
    }
}
