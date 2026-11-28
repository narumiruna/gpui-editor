// Run with Node 26 and the directory of SHA-512-pinned npm archives.
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {pinnedModules} from './pinned_modules.mjs';

function rows(tree, bytes) {
    const result = [];
    const pending = [[0, tree.topNode]];
    while (pending.length) {
        const [depth, node] = pending.pop();
        result.push([depth, node.type.id, node.name, bytes[node.from], bytes[node.to]]);
        for (let child = node.lastChild; child; child = child.prevSibling) {
            pending.push([depth + 1, child]);
        }
    }
    return result;
}

function byteOffsets(raw, editor) {
    const bytes = [0];
    let byte = 0;
    for (const [part] of raw.matchAll(editor ? /\r\n|[\s\S]/gu : /[\s\S]/gu)) {
        byte += Buffer.byteLength(part);
        // Map endpoints inside a surrogate pair to the scalar's end.
        if (part !== '\r\n' && part.length === 2) bytes.push(byte);
        bytes.push(byte);
    }
    return bytes;
}

assert.equal(process.argv.length, 3, 'usage: node lezer_grammar_resume.mjs <pinned archive directory>');
const scratch = mkdtempSync(join(realpathSync(tmpdir()), 'gpui-lezer-grammar-resume.'));
try {
    const {load, pins} = pinnedModules(resolve(process.argv[2]), scratch);
    const {lezerLanguage: {parser}} = await load('@codemirror/lang-lezer');
    const {DocInput} = await load('@codemirror/language');
    const {EditorState} = await load('@codemirror/state');
    const packages = Object.fromEntries([
        '@codemirror/lang-lezer', '@lezer/lezer', '@lezer/lr', '@lezer/common',
        '@codemirror/language', '@codemirror/state',
    ].map(name => [name, pins.get(name).version]));
    console.log(JSON.stringify({packages}));

    function emit(name, raw, marks) {
        for (const editor of [false, true]) {
            const input = editor ? new DocInput(EditorState.create({doc: raw}).doc) : raw;
            const bytes = byteOffsets(raw, editor);
            assert.equal(bytes.length, input.length + 1);
            const full = rows(parser.parse(input), bytes);
            const stages = [];
            for (let stop = 0; stop <= input.length; stop++) {
                // Public cutoffs never split scalars or editor CRLF pairs.
                if (bytes[stop] === bytes[stop + 1]) continue;
                const byte = bytes[stop];
                if (marks && stop !== input.length && !marks.includes(byte)) continue;
                // stopAt cannot move forward. Use a cold parse at each stop.
                const parse = parser.startParse(input);
                parse.stopAt(stop);
                let tree;
                while (!(tree = parse.advance())) {}
                const prefix = rows(tree, bytes);
                if (stop === input.length) assert.deepEqual(prefix, full);
                stages.push({stop: byte, rows: prefix});
            }
            console.log(JSON.stringify({name, raw, editor, stages}));
        }
    }

    emit('empty', '');
    emit('whitespace', ' \t\r\n ');
    emit('top-and-class', '@top Main { @digit+ }');
    emit('external-keywords', "@external propSource p from 'src'", [0, 4, 9, 10, 15, 22, 23, 27, 28, 30]);
    emit('specialization', '@top T { @specialize<Word, "if"> | @extend<Word, "else"> }', [0, 7, 9, 10, 20, 25, 37, 40, 48, 53]);
    emit('keyword-prefixes-are-not-specialized', '@topical T { @digitize }');
    emit('token-overrun-to-eof', '// a long comment 😀é');
    emit('literal-overrun', 'R { "long 😀é" }');
    emit('missing-closers', 'R { (a | b');
    emit('missing-body', '@top Foo');
    emit('wrong-closer', 'R { A ] B }');
    emit('astral-deletion', 'R { 😀 ? }');
    emit('astral-deletion-to-eof', '😀');
    emit('mixed-line-breaks', '// 😀\r\nR { "é" }\r// 日\rS {}');
} finally {
    rmSync(scratch, {recursive: true, force: true});
}
