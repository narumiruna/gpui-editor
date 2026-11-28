// node crates/codemirror/tests/fixtures/xml_brackets.mjs <pinned-archive-directory>
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {pinnedModules} from './pinned_modules.mjs';

const cases = [
    '<root title="[x)">z</root>',
    '<a><b><b/></b></a>',
    '<br><img/></br>',
    '<br><img/>',
    '< a \r\nx="" >x</ a \r>',
    '<ns:root a="{&amp;}" b=\'[]\'/>',
    '<x/>',
    '<a></A>',
    '<a1>t</a2>',
    '<a><b></a>',
    '</b>',
    '<a></b></a>',
    '<a></',
    '<a></a',
    '<a',
    '<',
    '<a x="(x',
    '<a x=[unquoted]></a>',
    '<a x=()/>',
    '<a x=( )/>',
    'é😀\r\n<漢😀 𐀀=(\r\n\t )/>',
    '<a x=[</>',
    'é😀\r\n<漢😀 x=\'é\'\r\n𐀀=\r\n[]/>',
    '<a x=(/>',
    '<a x="()"/>',
    '<a x=() />',
    '<a x=()/><b/>',
    '<r><a x=()/></r>',
    '<a &x=()/>',
    '<x /><a x = }\r\n[/>',
    '<a / >text</a>',
    '<a><',
    '<!-- <b> [x) -->',
    '<!-- ( --><!-- ) -->',
    '<!-- (x',
    '<![CDATA[<b> (x)]]>',
    '<![CDATA[<b> (x)]>',
    '<!DOCTYPE a [test]><a/>',
    '<!DOCTYPE a [<!ELEMENT a (#PCDATA)>]><a/>',
    '<?pi [a)?>',
    '<a>(</a><b>)</b>',
    '<a x="[" y="]">[</a>',
    '<a>(<!-- not a mate ) -->]</a>',
    '<a>&amp;[&#x20;]</a>',
    'é😀\r\n<漢😀 x="[a]\r[b)">漢\r</漢😀>',
    '<😀>😀(x)\r\n[x\ry]</😀>',
    '[](){}',
    '',
    ['<a>\n', ['😀', 5002], '\n</a>'],
    ['(', ['😀', 5002], ')'],
    ['(', ['😀', 4999], '\n)'],
    ['(', ['😀', 5000], '\n)'],
    ['(', ['é', 9998], '\r\n)'],
    ['(', ['é', 9998], '\r)'],
    ['(', ['é', 9999], '\r)'],
    ['(\n', ['😀', 5000], ')'],
];

const scratch = mkdtempSync(join(realpathSync(tmpdir()), 'gpui-xml-brackets.'));
try {
    const {load} = pinnedModules(resolve(process.argv[2]), scratch);
    const {xml} = await load('@codemirror/lang-xml');
    await load('@lezer/xml');
    const {bracketMatching, forceParsing, matchBrackets} = await load('@codemirror/language');
    const {EditorState, EditorSelection} = await load('@codemirror/state');
    const {ViewPlugin} = await load('@codemirror/view');

    let rendered;
    const matching = bracketMatching({renderMatch(hit) {
        assert.equal(rendered, null);
        rendered = hit;
        return [];
    }});
    // Run the actual plugin's render callback without a DOM. Its constructor
    // only needs the editor state.
    const plugin = matching.flat(Infinity).find(extension => extension instanceof ViewPlugin);
    assert(plugin);

    for (const source of cases) {
        const raw = typeof source === 'string' ? source
            : source.map(part => typeof part === 'string' ? part : part[0].repeat(part[1])).join('');
        const offsets = new Map([[0, 0]]);
        let pos = 0, byte = 0;
        for (const [part] of raw.matchAll(/\r\n|[^]/gu)) {
            pos += part === '\r\n' ? 1 : part.length;
            byte += Buffer.byteLength(part);
            offsets.set(pos, byte);
        }
        const view = {state: EditorState.create({doc: raw, extensions: [xml(), matching]}), dispatch(spec) {
            this.state = this.state.update(spec).state;
        }};
        assert(forceParsing(view, pos, 5000));
        const {state} = view;
        assert.equal(state.doc.length, pos);
        const range = ({from, to}) => {
            assert(offsets.has(from) && offsets.has(to));
            return [offsets.get(from), offsets.get(to)];
        };
        // For long inputs, probe both ends (including the tag-name handles).
        const hits = [...offsets].filter(([pos]) => typeof source === 'string'
            || pos <= 3 || pos >= state.doc.length - 4).map(([pos, byte]) => {
            const selected = state.update({selection: EditorSelection.cursor(pos)}).state;
            rendered = null;
            plugin.create({state: selected});
            const hit = matchBrackets(selected, pos, -1)
                || (pos > 0 && matchBrackets(selected, pos - 1, 1))
                || matchBrackets(selected, pos, 1)
                || (pos < state.doc.length ? matchBrackets(selected, pos + 1, -1) : null);
            assert.deepEqual(rendered, hit);
            return [byte, hit && [range(hit.start), hit.end ? range(hit.end) : null, hit.matched]];
        });
        console.log(JSON.stringify({source, hits}));
    }
} finally {
    rmSync(scratch, {recursive: true, force: true});
}
