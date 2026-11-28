// Run with Node and the directory of SHA-512-pinned npm archives.
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {pinnedModules} from './pinned_modules.mjs';

assert.equal(process.argv.length, 3, 'usage: node json_large_reduction.mjs <pinned archive directory>');
const scratch = mkdtempSync(join(realpathSync(tmpdir()), 'gpui-json-large-reduction.'));
try {
    const {load} = pinnedModules(resolve(process.argv[2]), scratch);
    const {json, jsonLanguage} = await load('@codemirror/lang-json');
    const {forceParsing, getIndentation, syntaxTree} = await load('@codemirror/language');
    const {EditorState} = await load('@codemirror/state');
    const strict = jsonLanguage.configure({strict: true}).parser;
    const view = state => ({state, dispatch(spec) { this.state = this.state.update(spec).state; }});
    const input = (depth, tail, closed) => '{"x":'.repeat(depth) + tail + '}'.repeat(closed ? depth : 0);
    function snapshot(editor, at) {
        assert(forceParsing(editor, editor.state.doc.length, 5000));
        return {
            shape: shape(syntaxTree(editor.state)),
            indents: at.map(pos => [pos, getIndentation(editor.state, pos) ?? null]),
        };
    }
    function check(source, at, strictOffset) {
        // These inputs are ASCII with LF, so UTF-16 offsets are byte offsets.
        assert.equal(Buffer.byteLength(source), source.length);
        assert.throws(() => strict.parse(source), new RegExp(`No parse at ${strictOffset}$`));
        const fresh = view(EditorState.create({doc: source, extensions: json()}));
        const result = snapshot(fresh, at);
        assert.deepEqual(result.shape, shape(jsonLanguage.parser.parse(source)));
        return result;
    }
    for (const [depth, closed] of [[3540, false], [3541, false], [12000, false], [12000, true]]) {
        const tail = closed ? '0' : '';
        const source = '['.repeat(depth) + tail + ']'.repeat(closed ? depth : 0);
        const at = 3200;
        const parse = jsonLanguage.parser.startParse(source);
        parse.stopAt(at);
        let tree;
        while (!(tree = parse.advance())) {}
        const prefix = {at, shape: shape(tree)};
        const positions = closed ? [depth, depth + 1, source.length] : [depth - 1, depth];
        console.log(JSON.stringify({container: 'Array', depth, tail, closed,
            ...check(source, positions, 2800), prefix}));
    }
    const line = [5810, 5811, 5812, 5813];
    const cases = [
        [1161, '0', true, [5805, 5806]],
        [1161, '0', false, [5805, 5806]],
        [1162, '0', true, [5809, 5810, 5811]],
        [1162, '0', false, [5809, 5810, 5811]],
        [1162, '\n0\n', true, line],
        [1162, '\n\n', true, line],
    ];
    const expected = new Map();
    for (const [depth, tail, closed, at] of cases) {
        const source = input(depth, tail, closed);
        const result = check(source, at, 4666);
        expected.set(source, result);
        console.log(JSON.stringify({depth, tail, closed, ...result}));
    }
    const filled = input(1162, '\n0\n', true);
    const empty = input(1162, '\n\n', true);
    const editor = view(EditorState.create({doc: filled, extensions: json()}));
    assert.deepEqual(snapshot(editor, line), expected.get(filled));
    editor.dispatch({changes: {from: 5811, to: 5812}});
    assert.equal(editor.state.doc.toString(), empty);
    assert.deepEqual(snapshot(editor, line), expected.get(empty));
    editor.dispatch({changes: {from: 5811, insert: '0'}});
    assert.equal(editor.state.doc.toString(), filled);
    assert.deepEqual(snapshot(editor, line), expected.get(filled));
} finally {
    rmSync(scratch, {recursive: true, force: true});
}

function shape(tree) {
    let depth = 0, count = 0, hash = 2166136261;
    let outerEnd = null, number = null;
    tree.iterate({enter(node) {
        if (depth === 1 && (node.name === 'Object' || node.name === 'Array')) outerEnd ??= node.to;
        if (node.name === 'Number') number ??= [depth, node.from, node.to];
        for (const byte of Buffer.from(`${depth}:${node.name}:${node.from}:${node.to};`)) {
            hash = Math.imul(hash ^ byte, 16777619) >>> 0;
        }
        ++depth;
        ++count;
    }, leave() { --depth; }});
    return {count, hash, outerEnd, number};
}
