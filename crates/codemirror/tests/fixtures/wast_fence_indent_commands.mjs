// Run with Node and a directory of local SHA-512-pinned npm archives.
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {pinnedModules} from './pinned_modules.mjs';

assert.equal(process.argv.length, 3, 'provide the directory of local npm archives');
const scratch = mkdtempSync(join(realpathSync(tmpdir()), 'wast-fence-indent.'));
try {
    const {load} = pinnedModules(resolve(process.argv[2]), scratch);
    const {EditorState} = await load('@codemirror/state');
    const {indentNodeProp, indentUnit, indentService} = await load('@codemirror/language');
    const {indentSelection, insertBlankLine, insertNewlineAndIndent, history, undo, undoDepth} =
        await load('@codemirror/commands');
    const {markdown} = await load('@codemirror/lang-markdown');
    const {wastLanguage: wast} = await load('@codemirror/lang-wast');
    const body = '(module\n  (func ())\n  (data "😀")\n)';
    for (const [name, label, command] of [
        ['enter', 'wat', insertNewlineAndIndent],
        ['blank', '', insertBlankLine],
        ['selection', 'wat', indentSelection],
        ['service', 'wat', insertNewlineAndIndent],
    ]) {
        const source = `Intro 😀\n\n\`\`\`${label}\n${body}\n\`\`\`\n\nTail.`;
        const first = source.indexOf('  (func');
        const last = source.indexOf('  (data');
        const at = name === 'blank' ? first + 2 : source.indexOf('()') + 1;
        const selection = name === 'selection' ? {anchor: last + 4, head: first + 1} : {anchor: at, head: at};
        const byte = offset => Buffer.byteLength(source.slice(0, offset));
        const calls = [];
        const configured = wast.configure({props: [indentNodeProp.add({App(cx) {
            const next = cx.continue();
            const before = cx.pos >= last ? cx.lineIndent(first) : 0;
            calls.push({range: [byte(cx.node.from), byte(cx.node.to)], pos: byte(cx.pos),
                unit: cx.unit, next, before, head: byte(cx.state.selection.main.head),
                simulated: cx.simulatedBreak == null ? null : byte(cx.simulatedBreak),
                double: !!cx.options.simulateDoubleBreak});
            assert(cx.state.doc.toString().endsWith('Tail.'));
            return cx.unit + 1 + (next ?? 0) + before;
        }})]});
        const stock = name === 'service' ? [indentService.of(() => 9)] : [];
        const initial = EditorState.create({doc: source, selection, extensions: [history(), indentUnit.of('   '),
            markdown({codeLanguages: name => name === 'wat' ? configured : null, defaultCodeLanguage: configured}), ...stock]});
        const view = {state: initial, dispatch(tr) { this.state = tr.state; }};
        view.dispatch = view.dispatch.bind(view);
        assert(command(view));
        const result = view.state.doc.toString();
        const after = offset => Buffer.byteLength(result.slice(0, offset));
        const main = view.state.selection.main;
        const depth = undoDepth(view.state);
        assert(undo(view));
        assert.equal(view.state.doc.toString(), source);
        assert.deepEqual(view.state.selection.main, initial.selection.main);
        console.log(JSON.stringify({name, source, body,
            selection: [byte(selection.anchor), byte(selection.head)], result,
            expected: [after(main.anchor), after(main.head)], depth, calls}));
    }
} finally {
    rmSync(scratch, {recursive: true, force: true});
}
