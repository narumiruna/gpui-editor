// From the repository root, with Node 22.14+ and the pinned archive directory:
// node crates/codemirror/tests/fixtures/markdown_language_presence.mjs <archive-root>
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {pinnedModules} from './pinned_modules.mjs';

const cases = [
    {name: 'root LF', raw: '# 😀\n\n```wast\n(module ;; 漢)\n```\n'},
    {name: 'quote CRLF', raw: '> > ```wast\r\n> > (module\r\n> >  (func))\r\n> > ```'},
    {name: 'quote CR', raw: '> ```json\r> {\r>   "漢":"😀"\r> }\r> ```'},
    {name: 'adjacent fences', raw: '```wast\r(module)\r\n```\n```json\r\n{}\r```'},
    {name: 'unclosed at EOF', raw: '```wast\n(module)'},
    {name: 'unclosed with final break', raw: '```json\r{}\r\n'},
    {name: 'zero-width and break-only', raw: '```wast\n```\n\n```json\r\n\r\n```\n\n> ```html\n> \r> ```'},
    {name: 'whitespace only', raw: '```wast\n  \n```'},
    {name: 'ordinary HTML', raw: '😀 a <i></i> z\n\n> <div>\r> 漢\r\n> </div>'},
    {name: 'indented fallback', raw: '    (module)\r\n\n```\r(func)\r```', mode: 'fallback'},
    {name: 'stock and unknown', raw: '```wast\n(module)\n```\n\n```unknown\ntext\n```', mode: 'stock'},
    {name: 'unresolved', raw: '```unknown\ntext\n```'},
    {name: 'fenced HTML with script', raw: '```html\r\n<script>let x=1;</script>\r\n```'},
    {name: 'ordinary HTML with script', raw: '<script>let x=1;</script>'},
    {name: 'accepted edits', raw: '# A\n\n> ```js\r\n> let x = "😀"\r> x\n> ```', edits: [
        ['# A', '# 漢😀'],
        ['```js', '```json'],
        ['let x = "😀"\r> x', '{}'],
        ['> {}\n> ```', '> {}'],
        ['{}', '{}\r\n> ```'],
        ['> {}\r\n', '> \r\n'],
        ['> \r\n', ''],
    ]},
];
const normalize = raw => raw.replace(/\r\n?/g, '\n');
const bytes = text => Buffer.byteLength(text);
const emit = row => console.log(JSON.stringify(row));

// Map normalized UTF-16 boundaries to original UTF-8 bytes. A surrogate
// interior has no byte offset. A CRLF midpoint has no editor offset.
function byteMap(raw) {
    const map = [0];
    let to = 0;
    for (const ch of raw.match(/\r\n|[^]/gu) ?? []) {
        if (ch.length === 2 && ch !== '\r\n') map.push(null);
        map.push(to += bytes(ch));
    }
    return map;
}

const root = realpathSync(mkdtempSync(join(tmpdir(), 'markdown-language-presence-')));
try {
    const {pins, load} = pinnedModules(resolve(process.argv[2]), root);
    const {EditorState} = await load('@codemirror/state');
    const {forceParsing, syntaxTree} = await load('@codemirror/language');
    const {markdown} = await load('@codemirror/lang-markdown');
    const {wastLanguage} = await load('@codemirror/lang-wast');
    const {jsonLanguage} = await load('@codemirror/lang-json');
    const {htmlLanguage} = await load('@codemirror/lang-html');
    const {javascriptLanguage} = await load('@codemirror/lang-javascript');
    const {IterMode, NodeProp} = await load('@lezer/common');
    const packages = ['@codemirror/state', '@codemirror/language', '@codemirror/lang-markdown',
        '@lezer/common', '@lezer/markdown', '@codemirror/lang-wast', '@codemirror/lang-json',
        '@codemirror/lang-html', '@codemirror/lang-javascript'].map(packageName => {
        const {name, version, tarball, integrity} = pins.get(packageName);
        return {name, version, tarball, integrity};
    });
    emit({capture: 'Markdown state language data and bracketed fence ranges', packages});

    function complete(state) {
        const view = {state, dispatch(spec) { this.state = this.state.update(spec).state; }};
        assert.ok(forceParsing(view, state.doc.length, 5000));
        return view.state;
    }

    function snapshot(state, raw, languages) {
        assert.equal(state.doc.toString(), normalize(raw));
        const map = byteMap(raw);
        const byte = at => {
            assert.equal(typeof map[at], 'number', `invalid normalized boundary ${at}`);
            return map[at];
        };
        const tree = syntaxTree(state), fences = [];
        tree.iterate({mode: IterMode.IgnoreMounts, enter(node) {
            const mount = node.tree?.prop(NodeProp.mounted);
            if (node.name !== 'FencedCode' || !mount) return;
            assert.ok(mount.bracketed);
            fences.push({owner: [byte(node.from), byte(node.to)],
                ranges: mount.overlay.map(({from, to}) => [byte(node.from + from), byte(node.from + to)])});
        }});
        const probes = Array.from({length: bytes(raw) + 2}, (_, at) => [at, null, null]);
        for (const [at, off] of map.entries()) {
            if (off == null) continue;
            for (const side of [-1, 1]) {
                const facet = state.languageDataAt('presenceFixture', at, side);
                const active = languages.filter(([language]) => language.isActiveAt(state, at, side));
                assert.equal(active.length, 1, `active language at ${off}, side ${side}`);
                assert.deepEqual(facet, [active[0][1]], `state facet at ${off}, side ${side}`);
                probes[off][side === -1 ? 1 : 2] = facet[0];
            }
        }
        return {fences, probes};
    }

    for (const {name, raw: initial, mode = 'controlled', edits = []} of cases) {
        const config = mode === 'stock' ? {} : {
            codeLanguages: info => ({wast: wastLanguage, json: jsonLanguage,
                html: htmlLanguage, js: javascriptLanguage})[info],
            ...(mode === 'fallback' ? {defaultCodeLanguage: wastLanguage} : {}),
        };
        const md = markdown(config);
        const languages = [[md.language, 'markdown'], [wastLanguage, 'wast'], [jsonLanguage, 'json'],
            [htmlLanguage, 'html'], [javascriptLanguage, 'javascript']];
        const extensions = [md, ...languages.map(([language, key]) => language.data.of({presenceFixture: key}))];
        const open = doc => complete(EditorState.create({doc, extensions}));
        let raw = initial, state = open(raw);
        emit({name, mode, raw, ...snapshot(state, raw, languages)});
        for (const [find, insert] of edits) {
            const start = raw.indexOf(find), end = start + find.length;
            assert.ok(start >= 0 && raw.lastIndexOf(find) === start, find);
            const before = normalize(raw.slice(0, start)), replaced = normalize(find);
            const edit = [bytes(raw.slice(0, start)), bytes(raw.slice(0, end)), insert];
            raw = raw.slice(0, start) + insert + raw.slice(end);
            state = complete(state.update({changes: {from: before.length,
                to: before.length + replaced.length, insert: normalize(insert)}}).state);
            const result = snapshot(state, raw, languages);
            assert.deepEqual(result, snapshot(open(raw), raw, languages), `${name}: fresh parse after ${find}`);
            emit({name, mode, raw, edit, ...result});
        }
    }
} finally {
    rmSync(root, {recursive: true, force: true});
}
