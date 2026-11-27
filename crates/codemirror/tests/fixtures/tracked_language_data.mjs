// From the repository root, run with the pinned archive and node_modules directory:
// node crates/codemirror/tests/fixtures/tracked_language_data.mjs <directory> \
//   > crates/codemirror/tests/fixtures/tracked_language_data.jsonl
import assert from 'node:assert/strict';
import {execFileSync} from 'node:child_process';
import {createHash} from 'node:crypto';
import {readFileSync, realpathSync} from 'node:fs';
import {findPackageJSON} from 'node:module';
import {join, resolve} from 'node:path';
import {pathToFileURL} from 'node:url';

const root = realpathSync(resolve(process.argv[2]));
const manifest = JSON.parse(readFileSync(new URL('../../../../docs/codemirror-upstream.json', import.meta.url)));
const pins = new Map([...manifest.packages, ...manifest.referenceDependencies].map(pin => [pin.name, pin]));
const verified = new Map();
function verify(name) {
    if (verified.has(name)) return verified.get(name);
    const pin = pins.get(name);
    const dir = join(root, name.replace('/', '__'));
    const archive = join(dir, pin.tarball.split('/').at(-1));
    const hash = createHash('sha512').update(readFileSync(archive)).digest('base64');
    assert.equal(`sha512-${hash}`, pin.integrity, name);
    const pkg = JSON.parse(readFileSync(join(dir, 'package/package.json')));
    assert.equal(pkg.name, name);
    assert.equal(pkg.version, pin.version);
    assert.equal(pkg.exports.import ?? pkg.exports['.'].import, './dist/index.js');
    for (const file of ['package.json', 'dist/index.js']) {
        const member = `package/${file}`;
        assert.deepEqual(readFileSync(join(dir, member)), execFileSync('tar', ['-xOf', archive, member]), `${name}/${file}`);
    }
    const entry = pathToFileURL(join(dir, 'package/dist/index.js'));
    verified.set(name, entry);
    for (const dep of Object.keys(pkg.dependencies ?? {}).filter(dep => pins.has(dep))) {
        const path = join(root, dep.replace('/', '__'), 'package/package.json');
        assert.equal(realpathSync(findPackageJSON(dep, entry)), path);
        verify(dep);
    }
    console.error(`${name} ${pin.version}: verified archive and resolved files`);
    return entry;
}

const {wast} = await import(verify('@codemirror/lang-wast'));
const {html} = await import(verify('@codemirror/lang-html'));
const {javascript} = await import(verify('@codemirror/lang-javascript'));
const {EditorState, StateEffect} = await import(verify('@codemirror/state'));
const {syntaxTreeAvailable} = await import(verify('@codemirror/language'));

const normalize = text => text.replace(/\r\n?/g, '\n');
const byteAt = (text, at) => Buffer.byteLength(text.slice(0, at));
const editorAt = (text, at) => {
    assert.ok(!(text[at - 1] === '\r' && text[at] === '\n'), 'position inside CRLF');
    return normalize(text.slice(0, at)).length;
};
const find = (text, needle) => {
    const at = text.indexOf(needle);
    assert.ok(at >= 0 && text.indexOf(needle, at + 1) < 0, needle);
    return at;
};
const replace = (needle, insert) => text => {
    const at = find(text, needle);
    return [at, at + needle.length, insert];
};
const keys = ['commentTokens', 'closeBrackets', 'wordChars', 'indentOnInput'];

function replay(language, support, source, points, steps) {
    let state = EditorState.create({doc: source});
    assert.deepEqual(state.languageDataAt('commentTokens', 0), []);
    state = state.update({effects: StateEffect.appendConfig.of(support)}).state;
    function emit(step) {
        assert.equal(state.doc.toString(), normalize(source));
        // These small documents have a complete tree from normal state updates.
        assert.ok(syntaxTreeAvailable(state, state.doc.length));
        const data = points(source).flatMap(at => [-1, 1].map(side => {
            const pos = editorAt(source, at);
            return {byte: byteAt(source, at), side, values: Object.fromEntries(keys.map(key => {
                const values = state.languageDataAt(key, pos, side);
                return [key, key === 'indentOnInput' ? values.length : values];
            }))};
        }));
        console.log(JSON.stringify({...step, source, data}));
    }
    emit({language});
    for (const step of steps) {
        if (step === 'selection') {
            const at = points(source)[1];
            state = state.update({selection: {anchor: editorAt(source, at)}}).state;
            emit({selection: byteAt(source, at)});
            continue;
        }
        const edits = step.map(edit => edit(source)).sort((a, b) => a[0] - b[0]);
        const changes = edits.map(([from, to, insert]) => [byteAt(source, from), byteAt(source, to), insert]);
        state = state.update({changes: edits.map(([from, to, insert]) => ({
            from: editorAt(source, from), to: editorAt(source, to), insert: normalize(insert),
        }))}).state;
        for (const [from, to, insert] of edits.reverse()) {
            source = source.slice(0, from) + insert + source.slice(to);
        }
        emit({changes});
    }
}

replay('wast', wast(), '(module\r\n  (func $f (result i32) i32.const 1)\r\n)\r\n',
    text => [0, find(text, '$f') + 2, text.length], [
        [() => [0, 0, ';; 😀\r\n']],
        'selection',
        [replace('i32.const 1', 'i32.const (')],
        [replace('i32.const (', 'i32.const 42')],
    ]);

replay('html', html(), '<p>😀</p>\r\n<script>/* probe */</script>\r\n',
    text => [0, find(text, 'probe')], [
        [() => [0, 0, '<!-- 新 -->\r\n']],
        [replace('<script>', '<script type="text/plain">')],
        [replace('<script type="text/plain">', '<style>'), replace('</script>', '</style>')],
    ]);

replay('jsx', javascript({jsx: true}), 'const el = <div>probe 😀</div>;\r\n',
    text => [0, find(text, 'probe') + 2], [
        [replace('probe 😀', '{probe}')],
        [replace('{probe}', 'probe')],
    ]);
