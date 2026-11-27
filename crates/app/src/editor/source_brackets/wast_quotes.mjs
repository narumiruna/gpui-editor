// From the repository root, regenerate with the pinned archives:
// node crates/app/src/editor/source_brackets/wast_quotes.mjs /private/tmp/gpui-upstream-exports.UVI6VVWZ \
//   > crates/app/src/editor/source_brackets/wast_quotes.jsonl
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {join, resolve} from 'node:path';
import {pathToFileURL} from 'node:url';

const root = resolve(process.argv[2]);
const manifest = JSON.parse(readFileSync(new URL('../../../../../docs/codemirror-upstream.json', import.meta.url)));
const pins = [...manifest.packages, ...manifest.referenceDependencies];
const dir = name => join(root, 'node_modules', name);
for (const name of ['@codemirror/lang-wast', '@codemirror/autocomplete', '@codemirror/language',
                    '@codemirror/state', '@codemirror/view', '@lezer/common', '@lezer/lr', '@lezer/highlight']) {
    const pkg = JSON.parse(readFileSync(join(dir(name), 'package.json')));
    assert.equal(pkg.name, name);
    assert.equal(pkg.version, pins.find(pin => pin.name === name).version);
}
const load = name => import(pathToFileURL(join(dir(name), 'dist/index.js')));
const {wast} = await load('@codemirror/lang-wast');
const {closeBrackets, insertBracket} = await load('@codemirror/autocomplete');
const {forceParsing, matchBrackets} = await load('@codemirror/language');
const {EditorState} = await load('@codemirror/state');

function snapshot(view) {
    assert.ok(forceParsing(view, view.state.doc.length, 5000));
    const {state} = view;
    const {anchor, head, empty} = state.selection.main;
    // Match the order of the bracket decorations. Convert UTF-16 positions
    // to byte offsets, including the original CRLF line endings.
    const hit = empty && (matchBrackets(state, head, -1)
        || (head > 0 && matchBrackets(state, head - 1, 1))
        || matchBrackets(state, head, 1)
        || (head < state.doc.length && matchBrackets(state, head + 1, -1)));
    const byte = pos => Buffer.byteLength(state.sliceDoc(0, pos));
    const range = ({from, to}) => [byte(from), byte(to)];
    return {
        body: state.sliceDoc(),
        selection: [byte(anchor), byte(head)],
        hit: hit ? [range(hit.start), hit.end ? range(hit.end) : null, hit.matched] : null,
    };
}

for (const [name, marked, actions] of [
    ['parsed opener', '(data |"字 😀 ([])" )', ['pair']],
    ['adjacent parsed opener', '(data "a"|"b")', ['pair']],
    ['inside parsed string', '(module\r\n (data "字 😀 | []"))', ['ordinary']],
    ['at parsed string end', '(data "a"| )', ['ordinary']],
    ['after whitespace outside string', '(data "a" |)', ['pair']],
    ['after supplementary word', '(data 𝟙| )', ['pair']],
    ['before supplementary word', '(data |𝟙 )', ['ordinary']],
    ['untracked closer', '(data "a|")', ['ordinary']],
    ['line comment', '(module\r\n ;; "字 | \r\n)', ['pair']],
    ['quote in line comment', '(module\r\n ;; |"字"\r\n)', ['ordinary']],
    ['block comment', '(module (; "字 | ;) )', ['pair']],
    ['quote in block comment', '(module (; |"字" ;) )', ['ordinary']],
    ['malformed opener', '(data |" )', ['ordinary']],
    ['space after malformed opener', '(data "x | )', ['pair']],
    ['string with an unknown escape', '(data "bad\\q | ")', ['ordinary']],
    ['top-level opener', '|"abc"', ['pair']],
    ['top-level string end', '"abc"|', ['ordinary']],
    ['top-level whitespace after string', '"abc" |', ['pair']],
    ['tracked closer before paren', '(data |)', ['pair', 'skip']],
    ['tracked closer at fence edge', '(module)\r\n|', ['pair', 'skip']],
    ['reverse selection', '(data |字😀|)', ['pair']],
]) {
    const [left, middle, right] = marked.split('|');
    const doc = left + middle + (right ?? '');
    const pos = text => text.replaceAll('\r\n', '\n').length;
    const selection = {anchor: pos(right == null ? left : left + middle), head: pos(left)};
    const extensions = [wast(), closeBrackets()];
    if (doc.includes('\r\n')) extensions.push(EditorState.lineSeparator.of('\r\n'));
    const view = {state: EditorState.create({doc, selection, extensions}), dispatch(spec) {
        this.state = this.state.update(spec).state;
    }};
    const before = snapshot(view);
    const after = actions.map(action => {
        const {state} = view;
        const tr = insertBracket(state, '"');
        assert.equal(tr ? (tr.newDoc.eq(state.doc) ? 'skip' : 'pair') : 'ordinary', action, name);
        view.state = (tr || state.update(state.replaceSelection('"'))).state;
        return snapshot(view);
    });
    console.log(JSON.stringify({name, before, after}));
}
