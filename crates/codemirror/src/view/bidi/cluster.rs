//! Cluster breaks for visual motion, from @marijn/find-cluster-break 1.0.4.
/*
MIT License

Copyright (C) 2024 by Marijn Haverbeke <marijn@haverbeke.berlin>

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in
all copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
THE SOFTWARE.
*/

use std::{ops::Range, sync::LazyLock};

// Unicode 17 Grapheme_Cluster_Break=Extend, as offset/length pairs in base 36.
// An empty field represents 1.
const EXTEND: &str = "lc,34,7n,7,7b,19,,,,2,,2,,,20,b,1c,l,g,,2t,7,2,6,2,2,,4,z,,u,r,2j,b,1m,9,9,,o,4,,9,,3,,5,17,3,1n,9,16,o,,x,1i,3,,i,,7,a,2,t,3,1k,,,7,2,2,2,3,9,,a,2,q,,2,3,1k,,,5,4,2,2,3,3,,u,2,3,,b,3,1k,,,8,,3,,3,k,2,m,6,,3,1k,,,7,2,2,2,3,7,3,a,2,u,,1n,5,3,3,,4,9,,14,5,1j,,,7,,3,,4,7,2,b,2,t,3,1k,,,7,,3,,4,7,2,b,2,f,,c,4,1j,2,,7,,3,,4,9,,a,2,t,3,1y,,4,6,,,,8,i,2,1p,,,8,c,8,2q,,,a,b,7,21,2,r,,,,,,4,2,1d,k,,2,5,b,,10,9,,2u,b,,6,n,4,4,3,g,4,d,,,3,6,,f,,jj,3,qa,4,s,3,t,2,u,2,1s,w,9,,19,3,,,39,2,y,,3a,c,4,c,63,5,1l,a,,,,,2,o,2,,1c,1a,2,c,k,5,1b,h,12,9,c,3,u,d,1k,e,1c,k,48,3,,l,4,,6,,2,3,5i,1s,ek,,5f,x,2da,3,3x,,2o,w,fe,6,2x,2,n9w,4,,a,w,2,28,2,7k,,3,,4,,n,5,4,,2b,2,1e,i,q,i,d,,12,8,p,d,18,4,1b,e,10,,1v,e,c,,8,2,1a,,1f,,,3,2,2,5,2,,,15,5,5,2,6k,8,,2,fn4,,kh,g,g,g,a6,2,gt,,6a,,45,5,1ae,3,,2,5,4,14,3,4,,4l,2,fx,4,1t,5,8t,2,25,6,1y,b,1d,4,3e,3,1h,f,15,,2,2,a,4,19,b,7,,1p,3,10,e,g,2,18,,c,3,1c,e,8,4,,2,2k,c,6,,2,,4d,c,l,4,1j,2,,7,2,2,2,3,9,,a,2,2,7,3,5,1v,9,,,2,,,4,,5,,,e,2,2a,i,n,,29,k,6j,7,2,9,r,2,2a,h,2y,d,2t,3,2,a,74,f,6t,6,,2,2,4,,,,2,3x,7,2,7,3,,s,a,14,7,,4,8,,9,b,1a,g,5i,8,5j,8,,8,2a,m,,e,3e,6,3,,,2,,7,,,1u,5,,2,,5,9n,4,9,2,,,1c,7,3,5,n,,44l,,6,f,8ug,i,1xc,5,1n,7,t4,,,1j,7,4,29,,b,2,f57,2,3mp,1a,2,n,f2,5,3,6,8,8,2,7,u,4,44,3,1iz,1j,4,1e,8,,e,,m,5,,f,11s,7,,h,2,7,,2,,5,2s,,4g,7,af,,1p,4,e4,4,72,2,6r,,2,,7,2,5,,d6,7,31,7,240,5";

static EXTENDING: LazyLock<Vec<Range<u32>>> = LazyLock::new(|| {
    let mut numbers = EXTEND.split(',').map(|s| {
        if s.is_empty() {
            1
        } else {
            u32::from_str_radix(s, 36).expect("valid encoded cluster table")
        }
    });
    let mut end = 0;
    std::iter::from_fn(|| {
        let start = end + numbers.next()?;
        end = start + numbers.next()?;
        Some(start..end)
    })
    .collect()
});

fn extending(ch: char) -> bool {
    let code = ch as u32;
    code >= 768
        && EXTENDING
            .get(EXTENDING.partition_point(|range| range.end <= code))
            .is_some_and(|range| range.contains(&code))
}

fn regional_indicator(ch: char) -> bool {
    matches!(ch as u32, 0x1f1e6..=0x1f1ff)
}

fn next_break(text: &str, pos: usize) -> usize {
    const ZWJ: char = '\u{200d}';
    let mut chars = text[pos..].char_indices();
    let Some((_, mut prev)) = chars.next() else {
        return pos;
    };
    for (i, next) in chars {
        if prev == ZWJ || next == ZWJ || extending(next) {
            prev = next;
        } else if !regional_indicator(next)
            || text[..pos + i]
                .chars()
                .rev()
                .take_while(|&ch| regional_indicator(ch))
                .count()
                .is_multiple_of(2)
        {
            return pos + i;
        }
    }
    text.len()
}

pub(super) fn find_break(text: &str, pos: usize, forward: bool) -> Option<usize> {
    if !text.is_char_boundary(pos) {
        return None;
    }
    if forward {
        return Some(next_break(text, pos));
    }
    let before = text[..pos].char_indices().rev();
    // Upstream's first backward probe starts two UTF-16 units before `pos`.
    // Skip the previous scalar only if it took one unit. Then probe each
    // earlier scalar once, since both halves of a surrogate pair probe its start.
    let skip = usize::from(
        before
            .clone()
            .next()
            .is_some_and(|(_, ch)| ch.len_utf16() == 1),
    );
    Some(
        before
            .skip(skip)
            .map(|(start, _)| next_break(text, start))
            .find(|&candidate| candidate < pos)
            .unwrap_or(0),
    )
}
