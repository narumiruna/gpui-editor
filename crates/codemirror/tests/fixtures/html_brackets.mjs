// node html_brackets.mjs <pinned-archive-directory> <scratch-directory>
import assert from 'node:assert/strict';
import {realpathSync} from 'node:fs';
import {pinnedModules} from './pinned_modules.mjs';

const pinned = pinnedModules(process.argv[2], realpathSync(process.argv[3]));
const {html} = await pinned.load('@codemirror/lang-html');
await pinned.load('@lezer/html');
const {forceParsing, matchBrackets} = await pinned.load('@codemirror/language');
const {EditorState} = await pinned.load('@codemirror/state');

const cases = [
    {source: '<div title="[a)">x</DIV>'},
    {source: '<div>z</span>'},
    {source: '</span>'},
    {source: '<a></b></a>'},
    {source: '<div><div></div></div>'},
    {source: '<div><span></div>'},
    {source: '<ul><li>a<li>b</ul>'},
    {source: '<div'},
    {source: '<div x="(x'},
    {source: '<div></div'},
    {source: '<div></di'},
    {source: '<div></'},
    {source: '<div><'},
    {source: '<x/>'},
    {source: '<x/>', options: {selfClosingTags: true}},
    {source: '<br><img/>'},
    {source: '<br/><script/>', options: {selfClosingTags: true}},
    {source: '<svg><path /><br/></svg>'},
    {source: '<p>x</p>', options: {matchClosingTags: false}},
    {source: '<div>z</span>', options: {matchClosingTags: false}},
    {source: '<a><b></a>', options: {matchClosingTags: false}},
    {source: '<script></script><style></style><textarea></textarea>'},
    {source: '<div><!-- <b> [x) --></div>'},
    {source: '<a title=(abc) data="[x" other="]">{x}</a>'},
    {source: '<a>(</a><b>)</b>'},
    {source: '<a x="[" style="f()" onclick="g()">]</a>', only: [0, 1, 2, 6, 7]},
    {source: 'é😀\r\n<漢😀 title="[a]\r[b)">漢\r</漢😀>'},
    {source: '<😀>x</😀>'},
    {source: ['<a>\n', ['😀', 5002], '\n</a>'], ends: [2, -1], scan: 1},
    {source: ['(', ['😀', 5002], ')']},
    {source: ['(', ['😀', 4999], '\n)']},
    {source: ['(', ['😀', 5000], '\n)']},
    {source: ['(', ['é', 9998], '\r\n)']},
    {source: ['(', ['é', 9998], '\r)']},
    {source: ['(', ['é', 9999], '\r)']},
    {source: ['(\n', ['😀', 5000], ')']},
];

for (const {source, options, only, ends, scan} of cases) {
    const raw = typeof source === 'string' ? source
        : source.map(part => typeof part === 'string' ? part : part[0].repeat(part[1])).join('');
    // Map normalized editor UTF-16 offsets to original UTF-8 byte boundaries.
    const utf16ToByte = new Map([[0, 0]]);
    let pos = 0, byte = 0;
    for (const [part] of raw.matchAll(/\r\n|[^]/gu)) {
        pos += part === '\r\n' ? 1 : part.length;
        byte += Buffer.byteLength(part);
        utf16ToByte.set(pos, byte);
    }
    const view = {state: EditorState.create({doc: raw, extensions: html(options)}), dispatch(spec) {
        this.state = this.state.update(spec).state;
    }};
    assert(forceParsing(view, view.state.doc.length, 5000));
    const {state} = view;
    assert.equal(pos, state.doc.length);
    const range = ({from, to}) => {
        assert(utf16ToByte.has(from) && utf16ToByte.has(to));
        return [utf16ToByte.get(from), utf16ToByte.get(to)];
    };
    const match = (pos, dir) => matchBrackets(state, pos, dir, {maxScanDistance: scan});
    const points = typeof source === 'string' ? only
        : (ends ?? [0, 1, -1, null]).map(at => at === null ? byte : at < 0 ? byte + at : at);
    const hits = [...utf16ToByte].filter(([, byte]) => !points || points.includes(byte))
        .map(([pos, byte]) => {
            const hit = match(pos, -1) || (pos > 0 && match(pos - 1, 1))
                || match(pos, 1) || (pos < state.doc.length && match(pos + 1, -1));
            return [byte, hit ? [range(hit.start), hit.end ? range(hit.end) : null, hit.matched] : null];
        });
    console.log(JSON.stringify({source, options, hits}));
}
