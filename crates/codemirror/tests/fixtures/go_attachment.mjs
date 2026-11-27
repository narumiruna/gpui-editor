// node crates/codemirror/tests/fixtures/go_attachment.mjs <archive-root>
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {pinnedModules} from './pinned_modules.mjs';

const root = realpathSync(mkdtempSync(join(tmpdir(), 'go-attachment-')));
try {
    const {load} = pinnedModules(resolve(process.argv[2]), root);
    const {go, goLanguage, localCompletionSource} = await load('@codemirror/lang-go');
    const {json} = await load('@codemirror/lang-json');
    const {Compartment, EditorState, Prec} = await load('@codemirror/state');
    const {syntaxTree} = await load('@codemirror/language');
    const {insertNewlineAndIndent} = await load('@codemirror/commands');
    const doc = 'package p\nfunc f(arg int) {\n  var local = 1\n  return local\n}\n';
    const slot = new Compartment();
    const sourceAt = state => state.languageDataAt('autocomplete', 0, 1);
    const emit = row => console.log(JSON.stringify(row));
    let state;
    function step(name, update) {
        const previous = state;
        state = update();
        const tree = syntaxTree(state);
        const sources = sourceAt(state);
        if (sources.length) {
            assert.equal(sources.length, 2);
            assert.equal(sources[1], localCompletionSource);
        }
        emit({name, active: goLanguage.isActiveAt(state, 0, 1), sources: sources.length,
            sameTree: previous ? tree === syntaxTree(previous) : false,
            root: tree.topNode.name});
    }
    const configure = extension => state.update({effects: slot.reconfigure(extension)}).state;
    const bare = goLanguage.extension;
    const bundle = go();
    step('bare', () => EditorState.create({doc, extensions: slot.of(bare)}));
    const retained = state;
    step('bundle', () => configure(bundle));
    step('select', () => state.update({selection: {anchor: 12}}).state);
    step('edit', () => state.update({changes: {from: doc.indexOf('local'), to: doc.indexOf('local') + 5, insert: 'inner'}}).state);
    step('fresh bundle', () => configure(go()));
    step('bare again', () => configure(bare));
    step('competing parser', () => configure([bundle, Prec.high(json())]));
    step('restore bundle', () => configure(bundle));
    step('detach', () => configure([]));
    assert.equal(sourceAt(retained).length, 0);
    assert.equal(retained.doc.toString(), doc);

    for (const [name, marked] of [
        ['empty block', 'package p\nfunc f() {|}'],
        ['spaced block', 'package p\nfunc f() { | }'],
        ['inside a string', 'package p\nvar s = "😀{|}"'],
    ]) {
        const at = marked.indexOf('|');
        const text = marked.replace('|', '');
        const offset = Buffer.byteLength(marked.slice(0, at));
        const editor = {
            state: EditorState.create({doc: text, selection: {anchor: at}, extensions: bundle}),
            dispatch(transaction) { editor.state = transaction.state; },
        };
        assert.ok(insertNewlineAndIndent(editor));
        const after = editor.state.doc.toString();
        emit({name, text, at: offset, after,
            head: Buffer.byteLength(after.slice(0, editor.state.selection.main.head))});
    }
} finally {
    rmSync(root, {recursive: true, force: true});
}
