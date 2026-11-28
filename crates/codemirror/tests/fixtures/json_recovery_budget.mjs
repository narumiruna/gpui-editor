// Run with Node and the directory of SHA-512-pinned npm archives.
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {pinnedModules} from './pinned_modules.mjs';

const objects = '{a:'.repeat(256);
const suffix = '\n,"suffix",false, null]';
const cases = [
    ['unexpected closers before more array values', '[' + '}'.repeat(256) + suffix],
    ['open objects with unquoted property names', objects],
    ['unclosed objects inside an array', '[' + objects + ']'],
    ['unclosed objects followed by more array values', '[' + objects + suffix],
    ...[2798, 2799, 2800, 2801].map(depth => [`balanced array at depth ${depth}`, {arrayDepth: depth}]),
    ...[932, 933, 934, 935, 936].map(depth => [`balanced object at depth ${depth}`, {objectDepth: depth}]),
];
assert.equal(process.argv.length, 3, 'usage: node json_recovery_budget.mjs <pinned archive directory>');
const scratch = mkdtempSync(join(realpathSync(tmpdir()), 'gpui-json-recovery-budget.'));
try {
    const {load} = pinnedModules(resolve(process.argv[2]), scratch);
    const {json, jsonLanguage} = await load('@codemirror/lang-json');
    const {forceParsing, foldable, getIndentation, syntaxTree} = await load('@codemirror/language');
    const {EditorState} = await load('@codemirror/state');
    for (const [name, input] of cases) {
        const source = typeof input === 'string' ? input : input.arrayDepth
            ? '['.repeat(input.arrayDepth) + ']'.repeat(input.arrayDepth)
            : '{"x":'.repeat(input.objectDepth) + '0' + '}'.repeat(input.objectDepth);
        // These inputs are ASCII and have no CRLF, so the offsets are bytes.
        assert.equal(Buffer.byteLength(source), source.length);
        const view = {state: EditorState.create({doc: source, extensions: json()}),
            dispatch(spec) { this.state = this.state.update(spec).state; }};
        assert(forceParsing(view, source.length, 5000));
        const tree = syntaxTree(view.state);
        assert.equal(tree.toString(), jsonLanguage.parser.parse(source).toString());
        let depth = 0, count = 0, hash = 2166136261;
        tree.iterate({enter(node) {
            for (const byte of Buffer.from(`${depth}:${node.name}:${node.from}:${node.to};`)) {
                hash = Math.imul(hash ^ byte, 16777619) >>> 0;
            }
            ++depth;
            ++count;
        }, leave() { --depth; }});
        const line = view.state.doc.line(1);
        const fold = foldable(view.state, line.from, line.to);
        console.log(JSON.stringify({name, ...(typeof input === 'string' ? {source} : input), count, hash,
            lineEnd: line.to, fold: fold ? [fold.from, fold.to] : null,
            indentAtEof: getIndentation(view.state, source.length) ?? null}));
    }
} finally {
    rmSync(scratch, {recursive: true, force: true});
}
