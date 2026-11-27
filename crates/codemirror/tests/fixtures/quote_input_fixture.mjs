import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {resolve, join} from 'node:path';
import {tmpdir} from 'node:os';
import {pinnedModules} from './pinned_modules.mjs';

export async function captureQuoteInput(language, cases, options, mode) {
    const scratch = mkdtempSync(join(realpathSync(tmpdir()), 'gpui-quote-input.'));
    try {
        const {load} = pinnedModules(resolve(process.argv[2]), scratch);
        const support = (await load(`@codemirror/lang-${language}`))[language];
        const {closeBrackets, insertBracket} = await load('@codemirror/autocomplete');
        const {forceParsing} = await load('@codemirror/language');
        const {EditorState} = await load('@codemirror/state');
        for (const [name, marked, key] of cases) {
            const [before, after] = marked.split('|');
            const source = before + after;
            const newline = source.includes('\r\n') ? '\r\n' : '\n';
            const extensions = [support(options), closeBrackets(), EditorState.lineSeparator.of(newline)];
            const view = {state: EditorState.create({doc: source, selection: {anchor: before.replaceAll('\r\n', '\n').length}, extensions}), dispatch(spec) {
                this.state = this.state.update(spec).state;
            }};
            assert(forceParsing(view, view.state.doc.length, 5000));
            const {state} = view;
            const tr = insertBracket(state, key);
            const done = (tr ?? state.update(state.replaceSelection(key))).state;
            const position = Buffer.byteLength(done.sliceDoc(0, done.selection.main.head));
            console.log(JSON.stringify({mode, name, source, head: Buffer.byteLength(before), key, after: done.sliceDoc(), position, handled: Boolean(tr)}));
        }
    } finally {
        rmSync(scratch, {recursive: true, force: true});
    }
}
