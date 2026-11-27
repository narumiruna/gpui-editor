// From the repository root: node crates/codemirror/tests/fixtures/wast_direct.mjs <archive-root>
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {pinnedModules} from './pinned_modules.mjs';

const root = realpathSync(mkdtempSync(join(tmpdir(), 'wast-direct-')));
try {
    const modules = pinnedModules(resolve(process.argv[2]), root);
    const {wastLanguage} = await modules.load('@codemirror/lang-wast');
    const {EditorState} = await modules.load('@codemirror/state');
    const {ensureSyntaxTree, syntaxTree} = await modules.load('@codemirror/language');
    const parser = wastLanguage.parser;
    const strict = wastLanguage.configure({strict: true}).parser;

    function byteOffsets(source, editor) {
        const offsets = [0];
        let byte = 0;
        for (const ch of source.match(editor ? /\r\n|[^]/gu : /[^]/gu) ?? []) {
            if (ch.length === 2 && (ch !== '\r\n' || !editor)) offsets.push(null);
            offsets.push(byte += Buffer.byteLength(ch));
        }
        return offsets;
    }

    function records(tree, source, editor = false) {
        const offsets = byteOffsets(source, editor);
        assert.equal(tree.length, offsets.length - 1);
        const rows = [];
        let depth = 0;
        tree.iterate({enter({name, from, to}) {
            assert.ok(Number.isInteger(offsets[from]) && Number.isInteger(offsets[to]));
            rows.push([depth++, name, offsets[from], offsets[to]]);
        }, leave() { depth--; }});
        return rows;
    }

    function hashRows(rows) {
        let hash = 2166136261;
        for (const byte of Buffer.from(JSON.stringify(rows))) hash = Math.imul(hash ^ byte, 16777619);
        return hash >>> 0;
    }

    function editorState(source) {
        return EditorState.create({doc: source, extensions: [wastLanguage]});
    }

    function capture(name, source, doc, change) {
        const raw = records(parser.parse(source), source);
        const editor = records(syntaxTree(doc), source, true);
        const hasError = raw.some(([, name]) => name === '⚠');
        let error = null;
        if (hasError) {
            assert.throws(() => strict.parse(source), e => {
                assert.ok(e instanceof SyntaxError);
                const [, at] = /^No parse at (\d+)$/.exec(e.message);
                error = Buffer.byteLength(source.slice(0, Number(at)));
                return true;
            });
        } else {
            assert.deepEqual(records(strict.parse(source), source), raw);
        }
        console.log(JSON.stringify({name, source, raw, editor, error, change}));
    }

    for (const [name, source] of [
        ['lone-cr', ';; 🦀\r(func)'],
        ['lone-cr-missing-close', '(module ;; 🦀\r  (func))'],
        ['crlf', '(module ;; 🦀\r\n  (func "é\r\n尾"))'],
        ['mixed', '(; café\r尾 ;)\r( $id i32 "🦀\r漢" )\n'],
    ]) capture(name, source, editorState(source));

    let source = '(func "🦀"\r\n  i32)';
    let state = editorState(source);
    capture('before-edit', source, state);
    for (const [name, remove, insert] of [
        ['remove-close', ')', ''],
        ['replace-type', 'i32', '@)'],
        ['repair', '@', 'i64'],
    ]) {
        const from = source.lastIndexOf(remove), to = from + remove.length;
        assert.ok(from >= 0);
        const prefix = at => source.slice(0, at);
        const change = [Buffer.byteLength(prefix(from)), Buffer.byteLength(prefix(to)), insert];
        const editorAt = at => prefix(at).replace(/\r\n?/g, '\n').length;
        state = state.update({changes: {from: editorAt(from), to: editorAt(to), insert}}).state;
        source = prefix(from) + insert + source.slice(to);
        capture(name, source, state, change);
    }

    const sample = ';; 🦀\r(func)\r\n(i32)';
    const expected = records(parser.parse(sample), sample);
    assert.deepEqual(records(parser.configure({top: 'Module'}).parse(sample), sample), expected);
    assert.throws(() => parser.configure({top: 'App'}), {name: 'RangeError', message: 'Invalid top rule name App'});
    assert.deepEqual(Object.keys(parser.dialects), []);
    assert.deepEqual(records(parser.configure({dialect: 'unknown'}).parse(sample), sample), expected);
    assert.deepEqual(strict.configure({strict: false}).parse('(').toString(), parser.parse('(').toString());

    for (const [name, repeat, count, tail] of [
        ['long', '(i32.const 0) ;; 🦀\r\n', 4096, ''],
        ['deep', '(', 2048, '"🦀\r\n尾"' + ')'.repeat(2048)],
    ]) {
        const source = repeat.repeat(count) + tail;
        const raw = records(parser.parse(source), source);
        assert.deepEqual(records(strict.parse(source), source), raw);
        assert.ok(raw.every(([, name]) => name !== '⚠'));
        console.log(JSON.stringify({name, bytes: Buffer.byteLength(source), nodes: raw.length, hash: hashRows(raw)}));
    }

    for (const [name, part, repeat] of [
        ['unclosed-comment', '(;', 256],
        ['unclosed-comment-crlf', '(;\r\n', 256],
        ['escaped-string', '"\\', 512],
    ]) {
        const source = part.repeat(repeat);
        const direct = records(parser.parse(source), source);
        const editor = editorState(source);
        const projected = records(ensureSyntaxTree(editor, editor.doc.length, Infinity), source, true);
        assert.deepEqual(direct, projected);
        console.log(JSON.stringify({name, part, repeat, lookahead: {nodes: direct.length, hash: hashRows(direct)}}));
    }

    for (const [part, depths] of [['(', [2799, 2800, 3000, 4000]], ['(i32 ', [1399, 1400, 1800]]]) {
        for (const depth of depths) {
            const source = part.repeat(depth) + ')'.repeat(depth);
            const rows = records(parser.parse(source), source);
            console.log(JSON.stringify({part, depth, permissive: {
                nodes: rows.length,
                errors: rows.filter(([, name]) => name === '⚠').length,
                hash: hashRows(rows),
            }}));
        }
    }

    for (const [name, prefix, part, depth, body, closes = depth] of [
        ['before-cut', '', '(', 2799, ''],
        ['at-cut', '', '(', 2800, ''],
        ['cut-on-opening', '', '(', 4096, ''],
        ['cut-after-type', '', '(', 2799, 'i32'],
        ['cut-multiple-frames', '', '(i32 ', 1401, ''],
        ['cut-with-space', ';; 😀\r\n', '( ', 2801, ''],
        ['cut-with-cr', ';; 😀\r', '( ', 2801, ''],
        ['accept-after-cut', '', '(i32 ', 1400, '', 1000],
    ]) {
        const source = prefix + part.repeat(depth) + body + ')'.repeat(closes);
        function result(parse, editor) {
            try {
                const rows = records(parse(), source, editor);
                return {hash: hashRows(rows), errors: rows.filter(([, name]) => name === '⚠').length};
            } catch (e) {
                if (!(e instanceof SyntaxError)) throw e;
                const [, at] = /^No parse at (\d+)$/.exec(e.message);
                const offset = byteOffsets(source, editor)[at];
                assert.ok(Number.isInteger(offset));
                return {offset};
            }
        }
        console.log(JSON.stringify({name, prefix, part, depth, body, closes,
            direct: result(() => strict.parse(source), false),
            editor: result(() => {
                const state = EditorState.create({doc: source, extensions: [wastLanguage.configure({strict: true})]});
                return ensureSyntaxTree(state, state.doc.length, Infinity);
            }, true),
        }));
    }
    {
        const prefixPart = '(func)\r', prefixCount = 96, comment = ';; comment 🦀\r\n';
        const prefix = prefixPart.repeat(prefixCount) + comment;
        const body = '"😀\r\n尾"';
        const depth = 2700, extra = 400;
        const shallow = prefix + '('.repeat(depth) + body + ')'.repeat(depth);
        const deep = prefix + '('.repeat(depth + extra) + body + ')'.repeat(depth + extra);
        const snapshot = (state, source) => {
            const tree = ensureSyntaxTree(state, state.doc.length, Infinity);
            const rows = records(tree, source, true);
            return {
                hash: hashRows(rows),
                errors: rows.filter(([, name]) => name === '⚠').length,
            };
        };
        const original = editorState(shallow);
        const before = snapshot(original, shallow);
        // Insert inside the leaf path so the old deep subtree cannot be reused whole.
        const from = prefix.replace(/\r\n?/g, '\n').length + depth;
        const edited = original.update({changes: [
            {from, insert: '('.repeat(extra)},
            {from: original.doc.length, insert: ')'.repeat(extra)},
        ]}).state;
        const cut = snapshot(edited, deep);
        const repaired = edited.update({changes: [
            {from, to: from + extra},
            {from: edited.doc.length - extra, to: edited.doc.length},
        ]}).state;
        assert.equal(before.errors, 0);
        assert.ok(cut.errors > 0);
        assert.deepEqual(snapshot(repaired, shallow), before);
        assert.deepEqual(snapshot(original, shallow), before);
        assert.deepEqual(snapshot(editorState(deep), deep), cut);
        const wrapped = original.update({changes: [
            {from: from - depth, insert: '('.repeat(extra)},
            {from: original.doc.length, insert: ')'.repeat(extra)},
        ]}).state;
        const wrapsReusedSubtree = snapshot(wrapped, deep);
        assert.equal(wrapsReusedSubtree.errors, 0);
        console.log(JSON.stringify({name: 'edit-across-depth-cut', prefixPart, prefixCount,
            comment, body, depth, extra, before, cut, wrapsReusedSubtree}));
    }
} finally {
    rmSync(root, {recursive: true, force: true});
}
