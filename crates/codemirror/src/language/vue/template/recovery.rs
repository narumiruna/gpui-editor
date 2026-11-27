use super::{HtmlNode, HtmlNodeKind::*, Range, Tag, Template, VueExpressionKind};

pub(super) fn content(template: &mut Template, mut source: &str, mut start: usize) {
    // The strict grammar cannot insert a missing `}}`. Keep expressions
    // before the error and let the last expression run to the HTML boundary.
    while let Some((text, rest)) = source.split_once("{{") {
        template.push(start..start + text.len(), Tag::Content);
        start += text.len() + 2;
        if let Some((expression, rest)) = rest.split_once("}}") {
            template.expression(expression, start, VueExpressionKind::Interpolation);
            start += expression.len();
            template.push(start..start + 2, Tag::Brace);
            start += 2;
            source = rest;
        } else {
            template.expression(rest, start, VueExpressionKind::Interpolation);
            return;
        }
    }
    template.push(start..start + source.len(), Tag::Content);
}

pub(super) fn attribute(template: &mut Template, source: &str, attr: HtmlNode<'_>) {
    for part in attr.children() {
        let range = part.range();
        match part.kind() {
            AttributeName => directive_name(template, &source[range.clone()], range.start),
            Is => template.push(range, Tag::Operator),
            UnquotedAttributeValue => template.push(range, Tag::Name),
            AttributeValue => quoted_value(template, source, part),
            _ => {}
        }
    }
}

fn directive_name(template: &mut Template, mut name: &str, mut start: usize) {
    if let Some(rest) = name.strip_prefix("v-") {
        let letters = rest.bytes().take_while(u8::is_ascii_alphabetic).count();
        if letters > 0 {
            let end = 2 + letters;
            template.push(start..start + end, Tag::Keyword);
            start += end;
            name = &name[end..];
        }
    }
    while let Some(ch) = name.chars().next() {
        let (len, tag) = if matches!(ch, ':' | '@') {
            (1, Some(Tag::Punctuation))
        } else if identifier_char(ch) {
            let len = name
                .char_indices()
                .find(|&(_, ch)| !identifier_char(ch))
                .map_or(name.len(), |(offset, _)| offset);
            (len, Some(Tag::Variable))
        } else {
            (ch.len_utf8(), None)
        };
        if let Some(tag) = tag {
            template.push(start..start + len, tag);
        }
        start += len;
        name = &name[len..];
    }
}

fn identifier_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_' || ('\u{c0}'..='\u{effff}').contains(&ch)
}

fn quoted_value(template: &mut Template, source: &str, part: HtmlNode<'_>) {
    let Range { start, end } = part.range();
    let bytes = &source.as_bytes()[start..end];
    if bytes.is_empty() {
        return;
    }
    let closed = bytes.len() > 1
        && bytes.first() == bytes.last()
        && !part
            .children()
            .next_back()
            .is_some_and(|node| node.kind() == Error);
    let body = start + 1..end - usize::from(closed);
    if body.is_empty() {
        template.push(start..end, Tag::Value);
    } else {
        template.push(start..body.start, Tag::Value);
        template.expression(
            &source[body.clone()],
            body.start,
            VueExpressionKind::Directive,
        );
        if closed {
            template.push(body.end..end, Tag::Value);
        }
    }
}
