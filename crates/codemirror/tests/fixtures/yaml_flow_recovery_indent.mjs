// From the repository root: node crates/codemirror/tests/fixtures/yaml_flow_recovery_indent.mjs <archive-root>
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {projectedOffsets} from './editor_line_offsets.mjs';
import {pinnedModules} from './pinned_modules.mjs';

const scratch = mkdtempSync(join(realpathSync(tmpdir()), 'gpui-yaml-flow-indent.'));
try {
    const {load} = pinnedModules(resolve(process.argv[2]), scratch);
    const {yaml} = await load('@codemirror/lang-yaml');
    const {forceParsing, getIndentation, IndentContext, syntaxTree} = await load('@codemirror/language');
    const {EditorState} = await load('@codemirror/state');
    const extensions = yaml();
    const normalize = source => source.replace(/\r\n?/g, '\n');
    const create = source => EditorState.create({doc: source, extensions});

    function snapshot(source, state) {
        const {units, byteAt} = projectedOffsets(source);
        const view = {state, dispatch(spec) { this.state = this.state.update(spec).state; }};
        assert.equal(state.doc.toString(), normalize(source));
        assert(forceParsing(view, units, 5000));
        const errors = [];
        syntaxTree(view.state).iterate({enter(node) {
            if (node.type.isError) errors.push([byteAt(node.from), byteAt(node.to)]);
        }});
        const indents = [[0, getIndentation(view.state, 0)]];
        let pos = 0;
        for (const [part] of source.matchAll(/\r\n|[^]/gu)) {
            pos += part === '\r\n' ? 1 : part.length;
            indents.push([byteAt(pos), getIndentation(view.state, pos)]);
        }
        const newlineIndent = getIndentation(new IndentContext(view.state, {simulateBreak: units}), units);
        return [view.state, {errors, indents, newlineIndent}];
    }

    function replay(name, source, steps = []) {
        let [state, expected] = snapshot(source, create(source));
        console.log(JSON.stringify({name, source, ...expected}));
        for (const [name, remove, insert] of steps) {
            assert(source.endsWith(remove), name);
            const start = source.length - remove.length;
            const from = projectedOffsets(source.slice(0, start)).units;
            const {units: to, byteAt} = projectedOffsets(source);
            const change = [byteAt(from), byteAt(to), insert];
            state = state.update({changes: {from, to, insert: normalize(insert)}}).state;
            source = source.slice(0, start) + insert;
            [state, expected] = snapshot(source, state);
            assert.deepEqual(expected, snapshot(source, create(source))[1], name);
            console.log(JSON.stringify({name, source, change, ...expected}));
        }
    }

    replay('unfinished implicit flow pair', 'root: [\r\n  😀,\r  名', [
        ['complete the implicit flow pair', '', ': 値'],
        ['close the sequence on another line', '', '\r\n]'],
        ['damage the closing token', ']', '@'],
        ['repair the closing token', '@', ']'],
    ]);
    replay('unfinished quoted key on the opening line', 'a: ["名😀\r\n  tail\r  b: next\n]');
    replay('unfinished implicit flow pair after a comment', 'root: [ # 名\r\n x,\r 💡@');
    replay('complete inner map in an unfinished outer sequence', 'outer: [\r\n  {a: 😀\r   }');
    replay('unfinished mapping without a final invalid token', 'outer: {\r  名: 😀,\r\n   ');
    replay('nonempty error at the end of a flow mapping', 'outer: {\r  名: 😀,\r\n   @');
} finally {
    rmSync(scratch, {recursive: true, force: true});
}
