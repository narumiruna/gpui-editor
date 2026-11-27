// Run with Node and the directory of SHA-512-pinned npm archives.
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {projectedOffsets} from './editor_line_offsets.mjs';
import {pinnedModules} from './pinned_modules.mjs';

const inputs = [
    ['empty', ''],
    ['lone CR', '// 😀\r@top Main { @digit+ }'],
    ['CRLF', '// 😀\r\n@top Main { "é" }'],
    ['specializers', '@external propSource p from "é"\n@top Main { @digit+ }'],
    ['missing brace', '@top Main { Word'],
    ['unexpected token', '// 😀\r\n@top Main { = }'],
    ['hidden incomplete rule', '// 😀\r@top Main {'],
    ['all four fold bodies', 'R { a }\n@tokens { a { "x" } }\n@skip { a }\n@precedence { a @left }'],
];

function treeRows(tree, byteAt) {
    const rows = [];
    tree.iterate({enter(node) { rows.push([node.name, byteAt(node.from), byteAt(node.to)]); }});
    return rows;
}

function result(parse, byteAt) {
    try {
        return {tree: treeRows(parse(), byteAt)};
    } catch (error) {
        const at = /^No parse at (\d+)$/.exec(error.message);
        if (!at) throw error;
        return {offset: byteAt(Number(at[1]))};
    }
}

assert.equal(process.argv.length, 3, 'usage: node lezer_language_handle.mjs <archive directory>');
const scratch = mkdtempSync(join(realpathSync(tmpdir()), 'gpui-lezer-handle.'));
try {
    const {load} = pinnedModules(resolve(process.argv[2]), scratch);
    const {lezer, lezerLanguage: stock} = await load('@codemirror/lang-lezer');
    const {Compartment, EditorState} = await load('@codemirror/state');
    const {syntaxTree} = await load('@codemirror/language');
    const emit = value => console.log(JSON.stringify(value));
    const strict = stock.configure({strict: true}, 'strict grammar');
    const recovered = strict.configure({strict: false}, '');
    assert.notEqual(lezer(), lezer());
    assert.equal(lezer().language, stock);
    assert.deepEqual(lezer().support, []);
    assert.equal(strict.data, stock.data);
    assert.equal(recovered.name, strict.name);
    assert.notEqual(strict.parser, stock.parser);
    assert.equal(strict.parser.topNode, stock.parser.topNode);
    assert.throws(() => stock.parser.configure({top: 'Body'}), /Invalid top rule name/);
    for (const [name, source] of inputs) {
        const rawByte = at => Buffer.byteLength(source.slice(0, at));
        const {byteAt} = projectedOffsets(source);
        const installed = language => syntaxTree(EditorState.create({doc: source, extensions: language}));
        const raw = result(() => stock.parser.parse(source), rawByte);
        assert.deepEqual(result(() => recovered.parser.parse(source), rawByte), raw);
        assert.deepEqual(result(() => stock.parser.configure({dialect: 'unknown'}).parse(source), rawByte), raw);
        emit({kind: 'parse', name, source, raw, strictRaw: result(() => strict.parser.parse(source), rawByte),
            editor: result(() => installed(stock), byteAt), strictEditor: result(() => installed(strict), byteAt)});
    }
    const source = '// 😀\r\n@top Main { Word }';
    const {byteAt} = projectedOffsets(source);
    const slot = new Compartment();
    const before = EditorState.create({doc: source, extensions: slot.of(strict)});
    const from = before.doc.length - 1;
    const initial = syntaxTree(before);
    const selected = before.update({selection: {anchor: 1}}).state;
    assert.equal(syntaxTree(selected), initial);
    const invalid = before.update({changes: {from, to: from + 1}});
    const edited = Buffer.from(source).subarray(0, byteAt(from)).toString();
    const rejected = result(() => syntaxTree(invalid.state), projectedOffsets(edited).byteAt);
    assert.equal(typeof rejected.offset, 'number');
    assert.equal(syntaxTree(before), initial);
    const next = before.update({effects: slot.reconfigure(recovered), changes: {from, to: from + 1}}).state;
    assert(stock.isActiveAt(next, 0));
    assert.notEqual(syntaxTree(next), initial);
    const repaired = next.update({changes: {from, insert: '}'}, effects: slot.reconfigure(strict)}).state;
    assert.equal(repaired.doc.toString(), before.doc.toString());
    emit({kind: 'edit', source, removeAt: byteAt(from), rejected,
        recovered: treeRows(syntaxTree(next), projectedOffsets(edited).byteAt),
        repaired: treeRows(syntaxTree(repaired), byteAt)});
} finally {
    rmSync(scratch, {recursive: true, force: true});
}
