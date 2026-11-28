// Run from the repository root with Node and the verified npm archive tree.
import assert from 'node:assert/strict';
import {existsSync, mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {projectInput, projectedOffsets} from './editor_line_offsets.mjs';
import {pinnedModules} from './pinned_modules.mjs';

assert.equal(process.argv.length, 3);
const archives = resolve(process.argv[2]);
const root = mkdtempSync(join(realpathSync(tmpdir()), 'sql-semicolon-recovery-'));
try {
    const {load, pins} = pinnedModules(archives, root);
    // Do not allow the shared loader to fetch a missing archive.
    for (const {name, version} of pins.values()) {
        const archive = join(archives, name.replace('/', '__'), `${name.split('/').at(-1)}-${version}.tgz`);
        assert(existsSync(archive), `missing local archive: ${archive}`);
    }
    const {MySQL, StandardSQL, sql} = await load('@codemirror/lang-sql');
    const {EditorState} = await load('@codemirror/state');
    const {foldable, getIndentation, syntaxTree} = await load('@codemirror/language');
    const {CompletionContext} = await load('@codemirror/autocomplete');

    function nodes(tree, byteAt) {
        const rows = [];
        function visit(node, depth) {
            rows.push([node.name, byteAt(node.from), byteAt(node.to), depth]);
            for (let child = node.firstChild; child; child = child.nextSibling) visit(child, depth + 1);
        }
        visit(tree.topNode, 0);
        return rows;
    }

    function observe(state, source, dialect, carets) {
        const {byteAt, units} = projectedOffsets(source);
        assert.equal(state.doc.length, units);
        const tree = nodes(syntaxTree(state), byteAt);
        assert.deepEqual(tree, nodes(dialect.language.parser.parse(state.doc.toString()), byteAt));
        const lines = Array.from({length: state.doc.lines}, (_, i) => {
            const {from, to} = state.doc.line(i + 1);
            const fold = foldable(state, from, to);
            return [byteAt(from), byteAt(to), getIndentation(state, from),
                fold ? [byteAt(fold.from), byteAt(fold.to)] : null];
        });
        const completions = carets.map(caret => {
            const [complete] = state.languageDataAt('autocomplete', caret);
            const r = complete(new CompletionContext(state, caret, false));
            return [byteAt(caret), r ? [byteAt(r.from), byteAt(r.to ?? caret),
                r.options.map(({label, type}) => [label, type])] : null];
        });
        return {source, tree, lines, completions};
    }

    for (const [name, dialect, source, anchor, probes, statementCount] of [
        ['exact', StandardSQL, 'SELECT (u.i)\nFROM users u;', '(u.i', ['(u.i'], 1],
        ['multiline-with-next-statement', StandardSQL, 'SELECT (1\n+ 2);\nSELECT 3;', '(1', [], 2],
        ['mysql-crlf-alias-scope', MySQL,
            '/* 字😀 */\r\nSELECT `u`.i;\r\nSELECT (`u`.i)\r\nFROM `users` AS `u`;\r\nSELECT `u`.i;', '(`u`.i',
            ['SELECT `u`.i', '(`u`.i', 'SELECT `u`.i'], 3],
    ]) {
        const extensions = sql({dialect, schema: {users: ['id']}}).extension;
        const original = EditorState.create({doc: source, extensions});
        const editorText = original.doc.toString();
        const editAt = editorText.indexOf(anchor) + anchor.length;
        const byteAt = projectedOffsets(source).byteAt(editAt);
        assert.equal(byteAt, Buffer.byteLength(source.slice(0, source.indexOf(anchor) + anchor.length)));
        if (name === 'mysql-crlf-alias-scope') {
            assert.notEqual(byteAt, editAt);
            assert.notEqual(source.indexOf(anchor) + anchor.length, editAt);
        }
        let previous = 0;
        const carets = probes.map(probe => {
            const start = editorText.indexOf(probe, previous);
            assert(start >= 0);
            return previous = start + probe.length;
        });
        const change = original.update({changes: {from: editAt, insert: ';'}});
        const afterSource = projectInput(source, change);
        const afterCarets = carets.map(caret => caret > editAt ? caret + 1 : caret);
        const repair = change.state.update({changes: {from: editAt, to: editAt + 1}});
        const before = observe(original, source, dialect, carets);
        const after = observe(change.state, afterSource, dialect, afterCarets);
        assert.deepEqual(after, observe(EditorState.create({doc: afterSource, extensions}), afterSource, dialect, afterCarets));
        assert.deepEqual(before, observe(repair.state, projectInput(afterSource, repair), dialect, carets));
        assert.equal(after.tree.filter(([kind]) => kind === 'Statement').length, statementCount);
        assert.deepEqual(after.tree.filter(([kind]) => kind === '⚠'), [['⚠', byteAt, byteAt + 1, 3]]);
        if (name === 'exact') {
            assert.equal(byteAt, 11);
            assert.deepEqual(after.completions, [[11, [10, 11, [['id', 'property']]]]]);
            assert.deepEqual(after.lines, [[0, 13, 0, [13, 27]], [14, 27, 2, null]]);
        }
        if (name === 'mysql-crlf-alias-scope') {
            assert.deepEqual(after.completions.map(([, result]) => result?.[2] ?? null),
                [null, [['id', 'property']], null]);
        }
        console.log(JSON.stringify({name, dialect: dialect === MySQL ? 'MySQL' : 'StandardSQL', editAt: byteAt, before, after}));
    }
} finally {
    rmSync(root, {recursive: true, force: true});
}
