// Run with Node and a directory of SHA-512-pinned npm archives.
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {projectedOffsets} from './editor_line_offsets.mjs';
import {pinnedModules} from './pinned_modules.mjs';

assert.equal(process.argv.length, 3, 'usage: node standalone_javascript_folds.mjs <archive directory>');
const scratch = mkdtempSync(join(realpathSync(tmpdir()), 'gpui-js-folds.'));
try {
    const {load} = pinnedModules(resolve(process.argv[2]), scratch);
    const {javascript} = await load('@codemirror/lang-javascript');
    const {foldEffect, foldGutter, foldable, foldedRanges, unfoldEffect} = await load('@codemirror/language');
    const {EditorState} = await load('@codemirror/state');
    const cases = [
        ['JavaScript', {}, ['JS', 'mjs', 'cjs'], ['function outer() {', '  const object = {',
            '    word: "猫😀",', '  };', '  return object;']],
        ['TypeScript', {typescript: true}, ['ts', 'mts', 'cts'], ['function outer() {', '  const object: {',
            '    word: string;', '  } = { word: "猫😀" };', '  return object;']],
        ['JSX', {jsx: true}, ['jsx'], ['function outer() {', '  return <Box>',
            '    <span>猫😀</span>', '  </Box>;']],
        ['TSX', {jsx: true, typescript: true}, ['tsx'], ['function outer(w: string) {', '  return <Box>',
            '    <span>{w}猫😀</span>', '  </Box>;']],
    ];
    for (const [mode, options, suffixes, body] of cases) for (const eol of ['\n', '\r\n']) {
        const source = ['// 猫😀', ...body, '}', 'outer();', ''].join(eol);
        let raw = source;
        let state = EditorState.create({doc: source, extensions: [javascript(options), foldGutter()]});
        const markers = () => {
            const {byteAt} = projectedOffsets(raw);
            return Array.from({length: state.doc.lines}, (_, i) => state.doc.line(i + 1)).flatMap(line => {
                const fold = foldable(state, line.from, line.to);
                return fold ? [[byteAt(line.from), byteAt(fold.from), byteAt(fold.to)]] : [];
            });
        };
        const folds = () => {
            const {byteAt} = projectedOffsets(raw);
            const result = [];
            for (const cursor = foldedRanges(state).iter(); cursor.value; cursor.next()) {
                result.push([byteAt(cursor.from), byteAt(cursor.to)]);
            }
            return result;
        };
        const before = markers();
        assert.equal(before.length, 2);
        const line = state.doc.line(2);
        state = state.update({effects: foldEffect.of(foldable(state, line.from, line.to))}).state;
        const folded = folds();
        state = state.update({changes: {from: 0, insert: '\n'}}).state;
        raw = eol + raw;
        const mapped = folds();
        assert.deepEqual(mapped, folded.map(range => range.map(at => at + eol.length)));
        const cursor = foldedRanges(state).iter();
        state = state.update({effects: unfoldEffect.of({from: cursor.from, to: cursor.to})}).state;
        assert.deepEqual(folds(), []);
        console.log(JSON.stringify({mode, suffixes, source, eol, before, folded, mapped, after: markers()}));
    }
} finally {
    rmSync(scratch, {recursive: true, force: true});
}
