// node crates/codemirror/tests/fixtures/language_data_stream_extension.mjs <archive-root>
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {projectedOffsets, projectInput} from './editor_line_offsets.mjs';
import {pinnedModules} from './pinned_modules.mjs';

assert.equal(process.argv.length, 3);
const root = mkdtempSync(join(realpathSync(tmpdir()), 'language-data-stream-extension-'));
try {
    const {load, pins} = pinnedModules(resolve(process.argv[2]), root);
    assert.equal(pins.get('@codemirror/language-data').version, '6.5.2');
    assert.equal(pins.get('@codemirror/language').version, '6.13.1');
    const {languages} = await load('@codemirror/language-data');
    const {StreamLanguage, getIndentation, indentUnit, syntaxTree} = await load('@codemirror/language');
    const {EditorState, Compartment} = await load('@codemirror/state');
    const {highlightTree, tags} = await load('@lezer/highlight');
    const bundles = await Promise.all(languages.map(description => description.load()));
    const streamEntries = [];
    const roots = new Set();
    for (const [i, support] of bundles.entries()) {
        if (!(support.language instanceof StreamLanguage)) continue;
        streamEntries.push(i);
        const state = EditorState.create({doc: 'x', extensions: support.extension});
        assert(support.language.isActiveAt(state, 0));
        assert(!roots.has(syntaxTree(state).type), languages[i].name);
        roots.add(syntaxTree(state).type);
    }

    const id = name => languages.findIndex(d => d.name === name);
    const configure = ids => ids.map(i => bundles[i].extension);
    const clj = id('Clojure'), cljs = id('ClojureScript'), asn1 = id('ASN.1');
    const slot = new Compartment();
    const names = ['keyword', 'string', 'number', 'comment'];
    const highlighter = {style: styles => names.find(name => styles.includes(tags[name])) ?? null};
    const tabSize = 8, indent = '   ';
    const initial = '\t(defn f []\r\n  "字😀")\r\n;; note';
    let source = initial;
    let state = EditorState.create({doc: source, extensions: [slot.of(configure([clj])),
        EditorState.tabSize.of(tabSize), indentUnit.of(indent)]});
    const records = [];
    const record = (step, previous, actions = {}) => {
        const {byteAt} = projectedOffsets(source);
        const at = state.doc.line(2).from;
        const tree = syntaxTree(state);
        const spans = [];
        highlightTree(tree, highlighter, (from, to, tag) => spans.push([byteAt(from), byteAt(to), tag]));
        records.push({step, ...actions, source, head: byteAt(state.selection.main.head),
            active: [clj, cljs, asn1].find(i => bundles[i].language.isActiveAt(state, at)) ?? null,
            reused: previous === null ? null : tree === previous,
            at: byteAt(at), indent: getIndentation(state, at), spans,
            comments: state.languageDataAt('commentTokens', at).map(value => value.line),
            brackets: state.languageDataAt('closeBrackets', at).map(value => value.brackets),
            hasDefn: state.languageDataAt('autocomplete', at).map(values => values.includes('defn')),
            indentTrigger: state.languageDataAt('indentOnInput', at).map(rule => rule.test('  }'))});
    };
    record('install', null);
    const change = (step, spec, actions) => {
        const previous = syntaxTree(state);
        const tr = state.update(spec);
        source = projectInput(source, tr);
        state = tr.state;
        record(step, previous, actions);
    };
    change('select', {selection: {anchor: 1}}, {select: 1});
    const reconfigure = (step, extensions) => change(step,
        {effects: slot.reconfigure(configure(extensions))}, {extensions});
    reconfigure('repeat one extension', [clj, clj]);
    reconfigure('different entry as fallback', [clj, cljs]);
    reconfigure('switch to the same recipe', [cljs, clj]);
    reconfigure('remove inactive entry', [cljs]);
    const from = state.doc.toString().indexOf('"字😀"'), to = from + '"字😀"'.length;
    const {byteAt} = projectedOffsets(source);
    change('edit Unicode token', {changes: {from, to, insert: '42'}},
        {edit: [byteAt(from), byteAt(to), '42']});
    reconfigure('detach', []);
    reconfigure('reattach', [cljs]);
    const before = syntaxTree(state);
    const asnSource = 'Example DEFINITIONS ::= BEGIN\r\n-- 字\r\nEND';
    const asnEdit = [0, Buffer.byteLength(source), asnSource];
    state = state.update({changes: {from: 0, to: state.doc.length, insert: asnSource},
        effects: slot.reconfigure(configure([asn1])), selection: {anchor: 0}}).state;
    source = asnSource;
    record('switch language and text', before, {extensions: [asn1], edit: asnEdit, select: 0});
    reconfigure('different recipe as fallback', [asn1, cljs]);
    console.log(`{\n  "streamEntries":${JSON.stringify(streamEntries)},\n  "initial":` +
        `${JSON.stringify({source: initial, extensions: [clj], tabSize, indent})},\n  "records":[\n    ` +
        `${records.map(record => JSON.stringify(record)).join(',\n    ')}\n  ]\n}`);
} finally {
    rmSync(root, {recursive: true, force: true});
}
