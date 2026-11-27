// Run with Node 26.10.0. Copy this file to a scratch directory containing
// node_modules from SHA-512-verified npm archives. Use state 6.7.6, view
// 6.44.0, and search 6.7.3 from docs/codemirror-upstream.json, and the runtime
// dependency pins (with hashes) in yaml_frontmatter_state.mjs.
// Write stdout to search_regexp_hyphen.jsonl.
import assert from 'node:assert/strict';
import {RegExpCursor} from '@codemirror/search';
import {Text} from '@codemirror/state';

for (const [id, pattern, source, ignoreCase = false] of [
    ['leading hyphen is a range start', '[--0]', '.'],
    ['hyphen is a range end', '([+--]+)', '*+,-./0'],
    ['hyphen starts a range after another range', '([a-z--0]+)', '🦀 A-./0é\nz0_!', true],
    ['two trailing hyphens are literals', '[a-z--]+', 'a-.'],
    ['negated range excludes the same hyphen interval', '([^--0]+)', '.🦀-\n/é0'],
    ['escaped hyphen remains a scalar range endpoint', String.raw`([\--0]+)`, '-./01'],
    ['set escape next to a literal hyphen', String.raw`[\d-]+`, 'a23-/'],
]) {
    const text = Text.of(source.split('\n'));
    const byte = pos => Buffer.byteLength(source.slice(0, pos));
    const visits = [];
    const cursor = new RegExpCursor(text, pattern, {ignoreCase, test(from) {
        visits.push(byte(from));
        return true;
    }});
    const matches = [...cursor].map(({from, to, match}) => [byte(from), byte(to), ...match]);
    console.log(JSON.stringify({kind: 'cursor', id, parts: [source], pattern, ignoreCase,
        range: [0, byte(text.length)], reject: [], visits, matches}));
}

const patterns = ['[a--b]', String.raw`[--\d]`, String.raw`[\w--]`, String.raw`[--\p{ASCII}]`];
for (const pattern of patterns) {
    assert.throws(() => new RegExpCursor(Text.of(['a']), pattern), SyntaxError);
}
console.log(JSON.stringify({kind: 'invalid', patterns}));
