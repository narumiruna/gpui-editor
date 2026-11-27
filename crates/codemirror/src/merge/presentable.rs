use super::{DiffChange, merge_adjacent, shift_left, shift_right};

pub(super) fn make_presentable(changes: &mut Vec<DiffChange>, a: &[char], b: &[char]) {
    let mut prev = 0;
    for i in 0..changes.len() {
        let next = changes.get(i + 1).map_or(a.len(), |ch| ch.a.start);
        let ch = &mut changes[i];
        let (len_a, len_b) = (ch.a.len(), ch.b.len());
        if (len_a != 0 && len_b != 0)
            || !within_units(&a[ch.a.clone()], 3)
            || !within_units(&b[ch.b.clone()], 3)
        {
            let mut before = word_before(a, ch.a.start, prev);
            let mut after = word_after(a, ch.a.end, next);
            if (len_a == 0 || len_b == 0) && before != 0 && after != 0 {
                let changed = if len_a == 0 {
                    &b[ch.b.clone()]
                } else {
                    &a[ch.a.clone()]
                };
                if changed.len() > before && changed.ends_with(&a[ch.a.start - before..ch.a.start])
                {
                    shift_left(ch, before);
                    before = 0;
                    after = word_after(a, ch.a.end, next);
                } else if changed.len() > after
                    && changed.starts_with(&a[ch.a.end..ch.a.end + after])
                {
                    shift_right(ch, after);
                    after = 0;
                    before = word_before(a, ch.a.start, prev);
                }
            }
            if before != 0 || after != 0 {
                ch.a = ch.a.start - before..ch.a.end + after;
                ch.b = ch.b.start - before..ch.b.end + after;
            } else if len_a == 0 || len_b == 0 {
                let (text, range) = if len_a == 0 { (b, &ch.b) } else { (a, &ch.a) };
                let changed = &text[range.clone()];
                if let Some(first) = changed.iter().position(|&c| c == '\n') {
                    let last = changed.iter().rposition(|&c| c == '\n').unwrap();
                    let tail = changed.len() - last - 1;
                    if first <= next - ch.a.end
                        && changed[..first] == text[range.end..range.end + first]
                    {
                        shift_right(ch, first);
                    } else if tail <= ch.a.start - prev
                        && changed[last + 1..] == text[range.start - tail..range.start]
                    {
                        shift_left(ch, tail);
                    }
                }
            }
        }
        prev = ch.a.end;
    }
    merge_adjacent(changes, |previous, next| {
        short_gap(a, previous.a.end, next.a.start) && short_gap(b, previous.b.end, next.b.start)
    });
}

fn short_gap(text: &[char], from: usize, to: usize) -> bool {
    from >= to || within_units(&text[from..to], 2)
}

fn within_units(text: &[char], max: usize) -> bool {
    text.len() <= max && text.iter().map(|ch| ch.len_utf16()).sum::<usize>() <= max
}

fn word(c: char) -> bool {
    // Match the ASCII test in the pinned implementation, including its lower
    // bound for digits. The Unicode test uses alphabetic and number categories.
    c.is_ascii_alphabetic() || ('1'..='9').contains(&c) || c >= '\u{c0}' && c.is_alphanumeric()
}

fn word_before(text: &[char], pos: usize, limit: usize) -> usize {
    if !text.get(pos).is_some_and(|&c| word(c)) {
        return 0;
    }
    let count = text[limit..pos]
        .iter()
        .rev()
        .take(8)
        .take_while(|&&c| word(c))
        .count();
    if count == 8 { 0 } else { count }
}

fn word_after(text: &[char], pos: usize, limit: usize) -> usize {
    if pos == 0 || !word(text[pos - 1]) {
        return 0;
    }
    let count = text[pos..limit]
        .iter()
        .take(8)
        .take_while(|&&c| word(c))
        .count();
    if count == 8 { 0 } else { count }
}
