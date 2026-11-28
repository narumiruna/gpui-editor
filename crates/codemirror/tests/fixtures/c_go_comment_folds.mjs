// Run with Node and a directory of SHA-512-pinned npm archives.
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {pinnedModules} from './pinned_modules.mjs';

assert.equal(process.argv.length, 3, 'usage: node c_go_comment_folds.mjs <archive directory>');
const scratch = mkdtempSync(join(realpathSync(tmpdir()), 'gpui-c-go-folds.'));
try {
    const {load} = pinnedModules(resolve(process.argv[2]), scratch);
    const {EditorState} = await load('@codemirror/state');
    const {foldable, foldNodeProp, syntaxTree} = await load('@codemirror/language');
    const {cpp} = await load('@codemirror/lang-cpp');
    const {go} = await load('@codemirror/lang-go');
    const sources = ['/**/', '/*\n  foo', '/*\r\n  猫\r\n', '/*\r\n  猫\r\na',
        '/*\r\n  猫😀', '/*\r\n  猫😀x', '/*\r\n  猫😀\r\n*/', '/*\r  猫😀x', '/* hi\n😀x'];
    function snapshot(source, state) {
        let unit = 0, byte = 0;
        const bytes = new Map([[0, 0]]);
        for (const [span] of source.matchAll(/\r\n|[^]/gu)) {
            unit += span === '\r\n' ? 1 : span.length;
            bytes.set(unit, byte += Buffer.byteLength(span));
        }
        const range = r => r && [bytes.get(r.from) ?? null, bytes.get(r.to) ?? null];
        const comments = [];
        syntaxTree(state).iterate({enter(ref) {
            if (ref.name === 'BlockComment') comments.push([range(ref), range(ref.type.prop(foldNodeProp)(ref.node, state))]);
        }});
        assert.equal(comments.length, 1);
        const lines = Array.from({length: state.doc.lines}, (_, i) => {
            const line = state.doc.line(i + 1);
            return [range(line), range(foldable(state, line.from, line.to))];
        });
        return {comments, lines};
    }
    for (const [language, extensions] of [['Cpp', cpp()], ['Go', go()]]) {
        for (const source of sources) {
            const state = EditorState.create({doc: source, extensions});
            console.log(JSON.stringify({language, source, ...snapshot(source, state)}));
        }
        const source = '/*\r\n  猫😀x*/';
        const state = EditorState.create({doc: source, extensions});
        const deleted = source.slice(0, -2);
        const edited = state.update({changes: {from: state.doc.length - 2, to: state.doc.length}}).state;
        console.log(JSON.stringify({language, source, ...snapshot(source, state), deleted: snapshot(deleted, edited),
            repaired: snapshot(source, edited.update({changes: {from: edited.doc.length, insert: '*/'}}).state)}));
    }
} finally {
    rmSync(scratch, {recursive: true, force: true});
}
