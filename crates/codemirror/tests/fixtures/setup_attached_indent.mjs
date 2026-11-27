// Run with Node and a directory of local SHA-512-pinned npm archives.
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {projectedOffsets, projectInput} from './editor_line_offsets.mjs';
import {pinnedModules} from './pinned_modules.mjs';

assert.equal(process.argv.length, 3, 'provide the directory of local npm archives');
const scratch = mkdtempSync(join(realpathSync(tmpdir()), 'setup-attached-indent.'));
try {
    const {load} = pinnedModules(resolve(process.argv[2]), scratch);
    const {EditorState, Compartment, Transaction} = await load('@codemirror/state');
    const {basicSetup, minimalSetup} = await load('codemirror');
    const {undo, redo, undoDepth, redoDepth} = await load('@codemirror/commands');
    const {json} = await load('@codemirror/lang-json');
    const source = '{\r\n  "名😀": true\r    ';
    const {units, byteAt} = projectedOffsets(source);
    for (const [preset, setup] of [['basic', basicSetup], ['minimal', minimalSetup]]) {
        for (const [mode, event] of [
            ['initial', 'input.type'], ['initial', 'input.complete'], ['initial', 'input.paste'],
            ['attach', 'input.type'], ['detach', 'input.type'], ['absent', 'input.type'],
        ]) {
            const slot = new Compartment();
            const initiallyAttached = mode === 'initial' || mode === 'detach';
            const view = {
                state: EditorState.create({doc: source, selection: {anchor: units},
                    extensions: [setup, slot.of(initiallyAttached ? json() : [])]}),
                dispatch(tr) { this.state = tr.state; },
            };
            view.dispatch = view.dispatch.bind(view);
            if (mode === 'attach' || mode === 'detach') {
                view.dispatch(view.state.update({effects: slot.reconfigure(mode === 'detach' ? [] : json())}));
            }
            const tr = view.state.update({...view.state.replaceSelection('}'), userEvent: event,
                annotations: Transaction.time.of(1000)});
            view.dispatch(tr);
            const result = projectInput(source, tr);
            const after = projectedOffsets(result);
            const history = () => [undoDepth(view.state), redoDepth(view.state)];
            const snapshot = positions => ({head: positions(view.state.selection.main.head), history: history()});
            const applied = snapshot(after.byteAt);
            assert(undo(view));
            assert.equal(view.state.doc.toString(), source.replace(/\r\n?/g, '\n'));
            const undone = snapshot(byteAt);
            assert(redo(view));
            assert.equal(view.state.doc.toString(), result.replace(/\r\n?/g, '\n'));
            const redone = snapshot(after.byteAt);
            console.log(JSON.stringify({preset, mode, event, source, result, applied, undone, redone}));
        }
    }
} finally {
    rmSync(scratch, {recursive: true, force: true});
}
