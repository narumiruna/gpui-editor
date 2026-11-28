// Run with Node and a directory of local SHA-512-pinned npm archives.
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {projectedOffsets, projectInput} from './editor_line_offsets.mjs';
import {pinnedModules} from './pinned_modules.mjs';

assert.equal(process.argv.length, 3, 'provide the directory of local npm archives');
const scratch = mkdtempSync(join(realpathSync(tmpdir()), 'setup-java-indent.'));
try {
    const {load} = pinnedModules(resolve(process.argv[2]), scratch);
    const {Compartment, EditorState, Transaction} = await load('@codemirror/state');
    const {basicSetup} = await load('codemirror');
    const {defineLanguageFacet, languageDataProp, syntaxTree} = await load('@codemirror/language');
    const {javaLanguage} = await load('@codemirror/lang-java');
    const {undo, redo, undoDepth, redoDepth} = await load('@codemirror/commands');
    const source = 'String s = """\r\n  胡😀\r\n    ';
    const {units, byteAt} = projectedOffsets(source);
    const program = javaLanguage.configure({top: 'Program'});
    const members = javaLanguage.configure({top: 'ClassContent'});
    const disabled = members.configure({props: [languageDataProp.add({ClassContent: defineLanguageFacet({})})]});
    const parsers = {program, members, disabled};
    for (const [initial, next, event] of [
        ['program', null, 'input.type'],
        ['members', null, 'input.type'],
        ['disabled', null, 'input.type'],
        ['members', 'program', 'input.type'],
        ['program', 'members', 'input.complete'],
        ['members', null, 'input.paste'],
    ]) {
        const slot = new Compartment();
        const view = {state: EditorState.create({doc: source, selection: {anchor: units},
            extensions: [basicSetup, slot.of(parsers[initial])]}), dispatch(tr) { this.state = tr.state; }};
        view.dispatch = view.dispatch.bind(view);
        const tr = view.state.update({...view.state.replaceSelection('}'), userEvent: event,
            effects: next ? [slot.reconfigure(parsers[next])] : [], annotations: Transaction.time.of(1000)});
        view.dispatch(tr);
        const result = projectInput(source, tr);
        const after = projectedOffsets(result).byteAt;
        const snapshot = byte => ({head: byte(view.state.selection.main.head),
            root: syntaxTree(view.state).topNode.name, history: [undoDepth(view.state), redoDepth(view.state)]});
        const applied = snapshot(after);
        assert(undo(view));
        assert.equal(view.state.doc.toString(), source.replace(/\r\n?/g, '\n'));
        const undone = snapshot(byteAt);
        assert(redo(view));
        const redone = snapshot(after);
        assert.equal(view.state.doc.toString(), result.replace(/\r\n?/g, '\n'));
        console.log(JSON.stringify({source, initial, next, event, result, applied, undone, redone}));
    }
} finally {
    rmSync(scratch, {recursive: true, force: true});
}
