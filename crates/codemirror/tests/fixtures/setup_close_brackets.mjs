// Run with Node and a directory of local SHA-512-pinned npm archives.
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {pinnedModules} from './pinned_modules.mjs';

assert.equal(process.argv.length, 3, 'provide the directory of local npm archives');
const scratch = mkdtempSync(join(realpathSync(tmpdir()), 'setup-close-brackets.'));
try {
    const {load} = pinnedModules(resolve(process.argv[2]), scratch);
    const {EditorState, Compartment} = await load('@codemirror/state');
    const {EditorView, basicSetup, minimalSetup} = await load('codemirror');
    const {runScopeHandlers} = await load('@codemirror/view');
    const {closeBrackets} = await load('@codemirror/autocomplete');
    for (const {preset, mode, source, select, input} of [
        {preset: 'basic', mode: 'off', source: '', select: [0, 0], input: ['(', 'Backspace']},
        {preset: 'minimal', mode: 'off', source: '', select: [0, 0], input: ['(', 'Backspace']},
        {preset: 'minimal', mode: 'initial', source: '', select: [0, 0], input: ['(', 'Backspace']},
        {preset: 'minimal', mode: 'initial', source: '', select: [0, 0], input: ['(', ')']},
        {preset: 'minimal', mode: 'attach', source: '😀', select: [4, 0], input: ['[']},
        {preset: 'minimal', mode: 'detach', source: '', select: [0, 0], input: ['(', ')']},
    ]) {
        const slot = new Compartment();
        const inputExtension = mode === 'initial' || mode === 'detach' ? closeBrackets() : [];
        const toUnit = offset => Buffer.from(source).subarray(0, offset).toString().length;
        const view = {state: EditorState.create({doc: source,
            selection: {anchor: toUnit(select[0]), head: toUnit(select[1])},
            extensions: [preset === 'basic' ? basicSetup : minimalSetup, slot.of(inputExtension)],
        }), dispatch(tr) { this.state = tr.state; }};
        view.dispatch = view.dispatch.bind(view);
        if (mode === 'attach' || mode === 'detach') {
            view.dispatch(view.state.update({effects: slot.reconfigure(mode === 'attach' ? closeBrackets() : [])}));
        }
        const steps = input.map(key => {
            if (key === 'Backspace') {
                const handled = runScopeHandlers(view, {type: 'keydown', key, keyCode: 8,
                    preventDefault() {}, stopPropagation() {}}, 'editor');
                assert(handled);
            } else {
                const {from, to} = view.state.selection.main;
                const insert = () => view.state.update({...view.state.replaceSelection(key), userEvent: 'input.type'});
                if (!view.state.facet(EditorView.inputHandler).some(handler => handler(view, from, to, key, insert))) {
                    view.dispatch(insert());
                }
            }
            const doc = view.state.doc.toString();
            const {anchor, head} = view.state.selection.main;
            const byte = at => Buffer.byteLength(doc.slice(0, at));
            return {key, doc, selection: [byte(anchor), byte(head)]};
        });
        console.log(JSON.stringify({preset, mode, source, select, steps}));
    }
} finally {
    rmSync(scratch, {recursive: true, force: true});
}
