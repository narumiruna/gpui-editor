// Replay with Node and the directory of SHA-512-pinned archives.
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {pinnedModules} from './pinned_modules.mjs';

const cases = [
    ['before space in a pair', '(| )'],
    ['after space in a pair', '( |)'],
    ['empty pair', '(|)'],
    ['spaces and a tab', '( \t|  )'],
    ['nested pair', '(func (| ))'],
    ['nested pair with a CRLF prefix', '(module\r\n  (func (|  )))'],
    ['token to the right', '(local |i32)'],
    ['token to the left', '(local i32| )'],
    ['block comment delimiters', '(; note | ;)'],
    ['delimiters on separate lines', '(|\r\n)'],
    ['comment between the delimiters', '( |(; comment ;) )'],
];
const newline = '\r\n';
const scratch = mkdtempSync(join(realpathSync(tmpdir()), 'gpui-wast-newline.'));
try {
    const {load} = pinnedModules(resolve(process.argv[2]), scratch);
    const {wast} = await load('@codemirror/lang-wast');
    const {forceParsing} = await load('@codemirror/language');
    const {insertNewlineAndIndent} = await load('@codemirror/commands');
    const {EditorState} = await load('@codemirror/state');
    for (const [name, marked] of cases) {
        const [before, after] = marked.split('|');
        const source = before + after;
        const view = {
            state: EditorState.create({doc: source, selection: {anchor: before.replaceAll(newline, '\n').length},
                extensions: [wast(), EditorState.lineSeparator.of(newline)]}),
            dispatch(spec) { this.state = this.state.update(spec).state; },
        };
        assert(forceParsing(view, view.state.doc.length, 5000));
        const {state} = view;
        let result;
        assert(insertNewlineAndIndent({state, dispatch(tr) { result = tr.state; }}));
        console.log(JSON.stringify({name, source, head: Buffer.byteLength(before), after: result.sliceDoc(),
            position: Buffer.byteLength(result.sliceDoc(0, result.selection.main.head))}));
    }
} finally {
    rmSync(scratch, {recursive: true, force: true});
}
