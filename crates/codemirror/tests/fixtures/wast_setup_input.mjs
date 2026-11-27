// Run with Node and the directory of SHA-512-pinned npm archives.
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {projectedOffsets, projectInput} from './editor_line_offsets.mjs';
import {pinnedModules} from './pinned_modules.mjs';

const cases = [
    ['parenthesis pairs and skips', '(module |)', ['(', ')']],
    ['square opener stays single', '(module |)', ['[']],
    ['brace opener stays single', '\r(module |)', ['{']],
    ['apostrophe stays single', '(module |)', ["'"]],
    ['double quote pairs and skips', '(module\r\n  (data |)\r\n)', ['"', '"']],
    ['inside a parsed string', '(module\r\n  (data "😀 | "))', ['"']],
    ['inside a line comment', '(module\r  ;; "é | \r)', ['"']],
    ['before a parsed string opener', '(module (data |"字"))', ['"']],
    ['surround a reverse selection', '(data |字😀|)', ['"']],
    ['delete a parenthesis pair', '(module (|))', [null]],
    ['delete only the square opener', '(module [|])', [null]],
    ['delete only the apostrophe opener', "(module '|' )", [null]],
    ['delete a double quote pair', '(data "|")', [null]],
];
assert.equal(process.argv.length, 3, 'usage: node wast_setup_input.mjs <pinned archive directory>');
const scratch = mkdtempSync(join(realpathSync(tmpdir()), 'gpui-wast-setup-input.'));
try {
    const {load} = pinnedModules(resolve(process.argv[2]), scratch);
    const {wast} = await load('@codemirror/lang-wast');
    const {closeBrackets, deleteBracketPair, insertBracket} = await load('@codemirror/autocomplete');
    const {deleteCharBackward} = await load('@codemirror/commands');
    const {forceParsing} = await load('@codemirror/language');
    const {EditorState} = await load('@codemirror/state');
    const units = text => text.replace(/\r\n|\r/g, '\n').length;
    for (const [name, marked, inputs] of cases) {
        const [left, middle, right] = marked.split('|');
        const source = left + middle + (right ?? '');
        const selection = {anchor: units(right == null ? left : left + middle), head: units(left)};
        const view = {state: EditorState.create({doc: source, selection, extensions: [wast(), closeBrackets()]}),
            dispatch(spec) { this.state = spec.state ?? this.state.update(spec).state; }};
        const from = projectedOffsets(source).byteAt;
        const initial = [from(selection.anchor), from(selection.head)];
        let body = source;
        const steps = [];
        for (const input of inputs) {
            assert(forceParsing(view, view.state.doc.length, 5000));
            let tr;
            if (input === null) {
                const target = {state: view.state, dispatch(transaction) { tr = transaction; }};
                assert(deleteBracketPair(target) || deleteCharBackward(target));
            } else {
                tr = insertBracket(view.state, input) || view.state.update(view.state.replaceSelection(input));
            }
            view.dispatch(tr);
            body = projectInput(body, tr);
            const byte = projectedOffsets(body).byteAt;
            const {anchor, head} = view.state.selection.main;
            steps.push({input, body, selection: [byte(anchor), byte(head)]});
        }
        console.log(JSON.stringify({name, source, initial, steps}));
    }
} finally {
    rmSync(scratch, {recursive: true, force: true});
}
