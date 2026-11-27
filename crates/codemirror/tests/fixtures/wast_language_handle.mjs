// Run with Node and the directory of SHA-512-pinned npm archives.
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {projectedOffsets} from './editor_line_offsets.mjs';
import {pinnedModules} from './pinned_modules.mjs';

assert.equal(process.argv.length, 3, 'usage: node wast_language_handle.mjs <pinned archive directory>');
const scratch = mkdtempSync(join(realpathSync(tmpdir()), 'gpui-wast-language-handle.'));
try {
    const {load} = pinnedModules(resolve(process.argv[2]), scratch);
    const {Compartment, EditorState, Prec} = await load('@codemirror/state');
    const {language, syntaxTree} = await load('@codemirror/language');
    const {wast, wastLanguage} = await load('@codemirror/lang-wast');
    const {json, jsonLanguage} = await load('@codemirror/lang-json');
    const emit = value => console.log(JSON.stringify(value));
    const slot = new Compartment();
    const renamed = wastLanguage.configure({}, 'renamed-wast');
    const strict = renamed.configure({strict: true}, 'strict-wast');
    const reset = strict.configure({strict: false}, 'recovered-wast');
    assert.notEqual(renamed.parser, wastLanguage.parser);
    assert.equal(renamed.data, wastLanguage.data);
    assert.equal(strict.data, wastLanguage.data);
    assert.equal(reset.data, wastLanguage.data);
    const factory = wast();
    assert.equal(factory.language, wastLanguage);
    assert.equal(factory.support.length, 0);
    const extension = [Prec.high(renamed.data.of({wordChars: '@'})),
        wastLanguage.data.of({wordChars: '$'}), slot.of(factory)];
    let raw = ';; 😀\r\n(module)\r';
    let state = EditorState.create({doc: raw, extensions: extension});
    let lastTree = null;
    function snapshot(step) {
        assert.equal(state.doc.toString(), raw.replace(/\r\n?/g, '\n'));
        const tree = syntaxTree(state);
        const {byteAt} = projectedOffsets(raw);
        const positions = [0, state.doc.length];
        const active = query => positions.map(at => query.isActiveAt(state, at));
        const regions = wastLanguage.findRegions(state).map(({from, to}) => [byteAt(from), byteAt(to)]);
        const head = byteAt(state.selection.main.head);
        let depth = 0;
        const records = [];
        if (tree.topNode.name === 'Module') tree.iterate({enter(node) {
            records.push([depth++, node.name, byteAt(node.from), byteAt(node.to)]);
        }, leave() { --depth; }});
        emit({kind: 'snapshot', step, source: raw, name: state.facet(language)?.name ?? null,
            root: tree.topNode.name, tree: records, reused: lastTree === tree,
            defaultActive: active(wastLanguage), configuredActive: active(renamed), regions,
            words: state.languageDataAt('wordChars', 0), head});
        lastTree = tree;
    }
    function configure(extension) {
        state = state.update({effects: slot.reconfigure(extension)}).state;
    }
    function append(text) {
        state = state.update({changes: {from: state.doc.length, insert: text}}).state;
        raw += text;
    }
    const retained = state;
    snapshot('initial');
    configure(wast());
    snapshot('same factory language');
    configure(renamed);
    snapshot('renamed configuration');
    configure(renamed.configure({}, ''));
    snapshot('distinct parser with the same name');
    state = state.update({selection: {anchor: 1}}).state;
    snapshot('selection only');
    configure(strict);
    snapshot('strict configuration');
    const error = (step, source, run) => {
        assert.throws(run, e => {
            assert(e instanceof SyntaxError);
            const match = /^No parse at (\d+)$/.exec(e.message);
            assert(match, e.message);
            emit({kind: 'error', step, source, at: projectedOffsets(source).byteAt(Number(match[1]))});
            return true;
        });
    };
    error('strict edit', raw + '(', () => state.update({changes: {from: state.doc.length, insert: '('}}).state);
    snapshot('strict edit left old tree');
    append('(func)');
    snapshot('strict valid edit');
    configure(reset);
    snapshot('reset strict');
    append('(');
    snapshot('recovery after reset');
    error('invalid strict install', raw, () => EditorState.create({doc: raw, extensions: strict}));
    error('invalid strict reconfiguration', raw, () => state.update({effects: slot.reconfigure(strict)}).state);
    snapshot('rejected reconfiguration left old tree');
    configure([Prec.high(json()), strict]);
    assert(jsonLanguage.isActiveAt(state, 0));
    snapshot('competing language wins');
    configure([]);
    snapshot('detached');
    state = retained;
    raw = ';; 😀\r\n(module)\r';
    snapshot('retained state');
} finally {
    rmSync(scratch, {recursive: true, force: true});
}
