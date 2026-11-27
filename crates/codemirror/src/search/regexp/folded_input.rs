use std::ops::Range;

/// ECMAScript's `iu` mode adds the long s and Kelvin sign to the ASCII word
/// class. All consuming atoms already use simple case folding, so replacing
/// just these two scalars only changes how ASCII boundary assertions see them.
/// Keep the offsets needed to return ranges and captures from the original input.
pub(super) struct FoldedInput {
    pub(super) text: String,
    ends: Vec<(usize, usize)>,
}

impl FoldedInput {
    pub(super) fn new(source: &str) -> Option<Box<Self>> {
        let mut special = source.match_indices(['ſ', 'K']).peekable();
        special.peek()?;
        let mut text = String::with_capacity(source.len());
        let mut ends = Vec::new();
        let mut previous = 0;
        for (start, value) in special {
            text.push_str(&source[previous..start]);
            text.push(if value == "ſ" { 's' } else { 'K' });
            previous = start + value.len();
            ends.push((previous, text.len()));
        }
        text.push_str(&source[previous..]);
        Some(Box::new(Self { text, ends }))
    }

    pub(super) fn folded_offset(&self, source: usize) -> usize {
        let index = self.ends.partition_point(|&(end, _)| end <= source);
        let (raw, folded) = self.previous(index);
        source - (raw - folded)
    }

    pub(super) fn source_range(&self, folded: Range<usize>) -> Range<usize> {
        let offset = |at| {
            let index = self.ends.partition_point(|&(_, end)| end <= at);
            let (raw, folded) = self.previous(index);
            at + (raw - folded)
        };
        offset(folded.start)..offset(folded.end)
    }

    fn previous(&self, index: usize) -> (usize, usize) {
        index.checked_sub(1).map_or((0, 0), |i| self.ends[i])
    }
}
