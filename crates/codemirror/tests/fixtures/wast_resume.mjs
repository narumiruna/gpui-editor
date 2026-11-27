// Run with Node and the directory of SHA-512-pinned npm archives.
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {pinnedModules} from './pinned_modules.mjs';

function summarize(tree, large) {
    const rows = [];
    const pending = [[0, tree.topNode]];
    while (pending.length) {
        const [depth, node] = pending.pop();
        rows.push([depth, node.name, node.from, node.to]);
        for (let child = node.lastChild; child; child = child.prevSibling) {
            pending.push([depth + 1, child]);
        }
    }
    let hash = 2166136261;
    for (const byte of Buffer.from(JSON.stringify(rows))) {
        hash = Math.imul(hash ^ byte, 16777619) >>> 0;
    }
    return {end: tree.length, nodes: rows.length, hash, ...(!large && {rows})};
}

assert.equal(process.argv.length, 3, 'usage: node wast_resume.mjs <pinned archive directory>');
const scratch = mkdtempSync(join(realpathSync(tmpdir()), 'gpui-wast-resume-pin.'));
try {
    const {load} = pinnedModules(resolve(process.argv[2]), scratch);
    const {EditorState} = await load('@codemirror/state');
    const {wastLanguage: {parser}} = await load('@codemirror/lang-wast');

    function emit(name, raw, stops, {editor = false, repeat = 1} = {}) {
        const doc = raw.repeat(repeat);
        const source = editor ? EditorState.create({doc}).doc.toString() : doc;
        // Lezer cannot raise stopAt. Use independent oracles for each target.
        const stages = stops.map(stop => {
            const parse = parser.startParse(source);
            parse.stopAt(stop);
            let tree;
            while (!(tree = parse.advance())) {}
            return {stop, ...summarize(tree, repeat > 1)};
        });
        console.log(JSON.stringify({name, raw, repeat, editor, stages, full: summarize(parser.parse(source), repeat > 1)}));
    }

    emit('audit-3000-5000', '(func $f)\n', [3000, 5000], {repeat: 12000, editor: true});
    emit('audit-long-token', '(func $longname)\n', [8, 15, 16]);
    emit('audit-string-astral', '"😀" (func $f)', [2]);
    emit('audit-unexpected-close', ')(func $f)\n', [0]);
    emit('malformed-astral', '(func 😀 x)\n', [5, 7, 8, 9, 10]);
    emit('astral-half-stop', '😀 (func $f)\n', [0, 1, 2]);
    for (const [name, eol] of [['lf', '\n'], ['crlf', '\r\n'], ['cr', '\r']]) {
        const raw = `(func ;;é${eol} $f)${eol}`;
        emit(`raw-${name}`, raw, [7]);
        emit(`editor-${name}`, raw, [7, 9, 11], {editor: true});
    }
} finally {
    rmSync(scratch, {recursive: true, force: true});
}
