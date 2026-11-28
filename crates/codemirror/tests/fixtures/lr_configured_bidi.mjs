// Run with Node and a directory of local SHA-512-pinned npm archives.
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {projectedOffsets, projectInput} from './editor_line_offsets.mjs';
import {pinnedModules} from './pinned_modules.mjs';

assert.equal(process.argv.length, 3, 'provide the directory of local npm archives');
const scratch = mkdtempSync(join(realpathSync(tmpdir()), 'lr-configured-bidi.'));
try {
    const {load} = pinnedModules(resolve(process.argv[2]), scratch);
    const {EditorState, Compartment} = await load('@codemirror/state');
    const {Direction, EditorView} = await load('@codemirror/view');
    const {bidiIsolates, forceParsing, syntaxTree} = await load('@codemirror/language');
    const {NodeProp} = await load('@lezer/common');
    const {jsonLanguage: json} = await load('@codemirror/lang-json');
    const {lezerLanguage: lezer} = await load('@codemirror/lang-lezer');
    const {rustLanguage: rust} = await load('@codemirror/lang-rust');
    const {cppLanguage: cpp} = await load('@codemirror/lang-cpp');
    for (const [stock, source, props] of [
        [json, '{\r\n"名😀": ["אב", "abc"]\r}', {Object: 'rtl', PropertyName: 'ltr', String: null, Array: 'auto'}],
        [lezer, '// 😀\r\n@top Main {\r "אב" "abc"\r\n}', {Body: 'rtl', Literal: 'ltr', RuleName: 'auto'}],
        [rust, '#!/bin/rust\r\n// 😀\rfn main() { /* comment */ let s = "אב"; let t = "abc"; let u = r#"raw"#; }',
            {Block: 'rtl', String: null, RawString: 'ltr', BlockComment: null}],
        [cpp, '#include <system>\r\n// 😀\rint main() { /* comment */ auto s = "אב"; auto t = "abc"; auto u = R"(raw)"; }',
            {CompoundStatement: 'rtl', String: null, RawString: 'ltr', BlockComment: null}],
    ]) {
        const configured = stock.configure({props: [NodeProp.isolate.add(type => props[type.name])]});
        const slot = new Compartment();
        const cases = [];
        for (const [step, input, always, switched, move, clipped] of [
            ['stock', source, true, false, false, false],
            ['configured', source, true, true, false, false],
            ['configured-clipped', source, true, true, false, 1],
            ['configured-fragmented', source, true, true, false, 2],
            ['configured-edited', source, true, true, true, false],
            ['configured-optimized', source, false, true, false, false],
            ['ascii-optimized', source.replace('אב', 'xy'), false, true, false, false],
        ]) {
            const isolation = bidiIsolates({alwaysIsolate: always});
            const [plugin] = isolation;
            const original = EditorState.create({doc: input, extensions: [slot.of(stock), isolation]});
            let state = switched ? original.update({effects: slot.reconfigure(configured)}).state : original;
            let updated = input;
            let edit = null;
            if (move) {
                const from = state.doc.toString().indexOf('abc');
                const changes = {from, to: from + 3, insert: 'xyz漢'};
                const {byteAt} = projectedOffsets(input);
                edit = [byteAt(from), byteAt(from + 3), changes.insert];
                const transaction = state.update({changes});
                updated = projectInput(input, transaction);
                state = transaction.state;
            }
            const {units, byteAt} = projectedOffsets(updated);
            const at = state.doc.toString().indexOf('abc');
            const pinned = clipped === 1 ? [[at, at + 1]] : clipped === 2 ? [[at, at + 1], [at + 2, at + 3]] : [[0, units]];
            let value;
            const view = {
                state,
                textDirection: Direction.LTR,
                visibleRanges: pinned.map(([from, to]) => ({from, to})),
                plugin: key => key === plugin ? value : null,
                dispatch(spec) { this.state = this.state.update(spec).state; },
            };
            assert(forceParsing(view, units, 5000));
            const ranges = [];
            let upstreamError = null;
            try {
                value = plugin.create(view);
            } catch (error) {
                assert.equal(step, 'configured-fragmented');
                assert.equal(error.message, 'Ranges must be added sorted by `from` position and `startSide`');
                upstreamError = error.message;
            }
            if (upstreamError) {
                for (const [from, to] of pinned) syntaxTree(view.state).iterate({from, to, enter(node) {
                    const dir = node.type.prop(NodeProp.isolate);
                    if (dir) ranges.push([byteAt(node.from), byteAt(node.to), dir]);
                }});
                ranges.sort((a, b) => a[0] - b[0]);
            } else {
                const marks = view.state.facet(EditorView.bidiIsolatedRanges)[0](view);
                assert.equal(view.state.facet(EditorView.outerDecorations)[0](view), marks);
                for (let mark = marks.iter(); mark.value; mark.next()) {
                    const {class: name, attributes, inclusive} = mark.value.spec;
                    assert.equal(name, 'cm-iso');
                    assert.equal(inclusive, true);
                    ranges.push([byteAt(mark.from), byteAt(mark.to), attributes.dir]);
                }
            }
            cases.push({step, input, updated, always, switched, edit, visible: pinned.map(range => range.map(byteAt)), ranges, upstreamError});
        }
        console.log(JSON.stringify({name: stock.name, props, cases}));
    }
} finally {
    rmSync(scratch, {recursive: true, force: true});
}
