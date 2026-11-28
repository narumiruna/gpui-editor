// Run with Node and the directory of SHA-512-pinned npm archives.
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

assert.equal(process.argv.length, 3, 'usage: node json_resume.mjs <pinned archive directory>');
const scratch = mkdtempSync(join(realpathSync(tmpdir()), 'gpui-json-resume-pin.'));
try {
    const {load} = pinnedModules(resolve(process.argv[2]), scratch);
    const {jsonLanguage: {parser}} = await load('@codemirror/lang-json');
    const {DocInput} = await load('@codemirror/language');
    const {EditorState} = await load('@codemirror/state');

    function emit(name, raw, stops, editor = false) {
        const input = editor ? new DocInput(EditorState.create({doc: raw}).doc) : raw;
        // Index UTF-16 boundaries by their original UTF-8 offsets. A pin
        // endpoint inside an astral scalar projects to the end of that scalar.
        const bytes = [0];
        let byte = 0;
        for (const [part] of raw.matchAll(editor ? /\r\n|[\s\S]/gu : /[\s\S]/gu)) {
            byte += Buffer.byteLength(part);
            if (part !== '\r\n' && part.length === 2)
                bytes.push(byte);
            bytes.push(byte);
        }
        assert.equal(bytes.length, input.length + 1);
        const full = rows(parser.parse(input), bytes);
        stops ??= Array.from(bytes.keys()).filter(i => bytes[i] !== bytes[i + 1]);
        assert.equal(stops.at(-1), input.length, name);
        const stages = stops.map(stop => {
            // stopAt cannot move forward. Each cutoff needs a cold oracle.
            const parse = parser.startParse(input);
            parse.stopAt(stop);
            let tree;
            while (!(tree = parse.advance())) {}
            const prefix = rows(tree, bytes);
            if (stop === input.length)
                assert.deepEqual(prefix, full);
            return {stop: bytes[stop], rows: prefix};
        });
        console.log(JSON.stringify({name, raw, editor, stages}));
    }

    emit('empty', '', null);
    emit('token-overrun-at-eof', '"😀longé"', null);
    emit('skipped-at-eof', 'true \t', null);
    emit('nested-containers', '{"x":[true, -15.4e-2]}', [0, 2, 4, 5, 7, 9, 11, 12, 14, 19, 20, 21, 22]);
    emit('missing-value-and-closers', '{"x": [true, {"y":', null);
    emit('unexpected-token', '[1, } true, 2]', null);
    emit('missing-comma', '[1 2 "hi" true]', null);
    emit('invalid-key-and-value', '{oops: tru, "x": null}', [0, 1, 5, 8, 10, 11, 12, 15, 20, 21, 22]);
    emit('astral-deletion', '[😀, "é"]', null);
    emit('leading-errors', '}😀x [1]', null);
    emit('stray-values', 'true false null', null);
    const lines = '[\r\n"😀é",\r\n }\r1\r\n]';
    emit('raw-mixed-lines', lines, null);
    emit('editor-mixed-lines', lines, null, true);
    const stringBreaks = '"a\r\né\rb"';
    emit('raw-string-breaks', stringBreaks, null);
    emit('editor-string-breaks', stringBreaks, null, true);
} finally {
    rmSync(scratch, {recursive: true, force: true});
}
