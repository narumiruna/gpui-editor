// Run with Node and a directory of SHA-512-pinned npm archives.
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {projectedOffsets} from './editor_line_offsets.mjs';
import {pinnedModules} from './pinned_modules.mjs';

assert.equal(process.argv.length, 3, 'usage: node javascript_comment_folds.mjs <archive directory>');
const scratch = mkdtempSync(join(realpathSync(tmpdir()), 'gpui-js-comment-folds.'));
try {
    const {load} = pinnedModules(resolve(process.argv[2]), scratch);
    const {javascript} = await load('@codemirror/lang-javascript');
    const {foldable, foldNodeProp, syntaxTree} = await load('@codemirror/language');
    const {EditorState} = await load('@codemirror/state');
    const cases = [
        ['unclosed ASCII', '/* comment\n  foo'],
        ['unclosed after expression', 'x\n/*\nfoo\n*'],
        ['unclosed with two BMP scalars', '/*\nline 猫犬'],
        ['unclosed with supplementary scalar', '/*\nline 😀'],
        ['unclosed with split surrogate', '/*\nline 😀x'],
        ['unclosed with trailing CRLF', '/*\r\n  猫\r\n'],
        ['unclosed ending after trailing CRLF', '/*\r\n  猫\r\na'],
        ['closed with CRLF and supplementary scalar', '/*\r\n  猫😀\r\n*/'],
        ['empty', '/**/'],
    ];
    const extensions = javascript();
    const capture = (source, state) => {
        const {byteAt} = projectedOffsets(source);
        const position = at => /[\ud800-\udbff][\udc00-\udfff]/.test(state.sliceDoc(Math.max(0, at - 1), at + 1))
            ? null : byteAt(at);
        const range = ({from, to}) => [position(from), position(to)];
        const nodes = [];
        syntaxTree(state).iterate({enter: ({node}) => {
            if (node.name !== 'BlockComment') return;
            nodes.push(range(node.type.prop(foldNodeProp)(node, state)));
        }});
        const lines = Array.from({length: state.doc.lines}, (_, index) => state.doc.line(index + 1)).map(line => {
            const candidate = foldable(state, line.from, line.to);
            return [byteAt(line.from), candidate ? range(candidate) : null];
        });
        assert.equal(nodes.length, 1);
        return {nodes, lines};
    };
    for (const [name, source] of cases) {
        const state = EditorState.create({doc: source, extensions});
        console.log(JSON.stringify({name, source, ...capture(source, state)}));
    }
    const source = 'const x = 1;\r\n/*\r\n  猫😀\r\n*/';
    const state = EditorState.create({doc: source, extensions});
    const edited = state.update({changes: {from: state.doc.length - 2, to: state.doc.length}}).state;
    const changed = source.slice(0, -2);
    const repaired = edited.update({changes: {from: edited.doc.length, insert: '*/'}}).state;
    console.log(JSON.stringify({name: 'delete and restore closer', source, ...capture(source, state),
        edit: capture(changed, edited), repair: capture(source, repaired)}));
} finally {
    rmSync(scratch, {recursive: true, force: true});
}
