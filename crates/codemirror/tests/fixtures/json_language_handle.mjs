// Run with Node and a directory of local SHA-512-pinned npm archives.
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {projectedOffsets} from './editor_line_offsets.mjs';
import {pinnedModules} from './pinned_modules.mjs';

assert.equal(process.argv.length, 3, 'provide the directory of local npm archives');
const scratch = mkdtempSync(join(realpathSync(tmpdir()), 'json-language-handle.'));
try {
    const {load} = pinnedModules(resolve(process.argv[2]), scratch);
    const {json, jsonLanguage: stock, jsonParseLinter} = await load('@codemirror/lang-json');
    const {EditorState, Compartment} = await load('@codemirror/state');
    const {syntaxTree} = await load('@codemirror/language');
    const strict = stock.configure({strict: true}, 'strict JSON');
    const recovery = strict.configure({strict: false}, '');
    assert.notEqual(json(), json());
    assert.equal(json().language, stock);
    assert.deepEqual(json().support, []);
    assert.notEqual(strict.parser, stock.parser);
    assert.equal(strict.data, stock.data);
    assert.equal(recovery.name, strict.name);
    assert.equal(strict.parser.topNode.name, 'JsonText');
    assert.throws(() => stock.parser.configure({top: 'Object'}), /Invalid top rule name/);
    const rows = (tree, byteAt) => {
        const nodes = [];
        tree.iterate({enter(node) { nodes.push([node.name, byteAt(node.from), byteAt(node.to)]); }});
        return nodes;
    };
    const result = (run, byteAt) => {
        try {
            return {tree: rows(run(), byteAt)};
        } catch (error) {
            const at = /^No parse at (\d+)$/.exec(error.message);
            if (!at) throw error;
            return {offset: byteAt(Number(at[1]))};
        }
    };
    const valid = '{\r\n  "😀": [\r0,true,null]\r\n}';
    for (const [name, source] of [
        ['empty', ''], ['whitespace', '\r\n\r\t'], ['mixed valid', valid],
        ['trailing comma', '{\r\n"😀":0,\r}'], ['missing colon', '\r\n{"名😀" 1}'],
        ['lone CR missing colon', '\r{"名😀" 1}'],
        ['missing separator', '[\r\n"😀"\r false]'], ['missing closer', '{\r\n"😀": [\r0]'],
        ['bad CRLF in string', '[\r\n"😀\r\nx",0]'], ['bad lone CR in string', '[\r\n"😀\rx",0]'],
    ]) {
        const raw = at => Buffer.byteLength(source.slice(0, at));
        const {byteAt} = projectedOffsets(source);
        const install = language => syntaxTree(EditorState.create({doc: source, extensions: language}));
        const permissive = result(() => stock.parser.parse(source), raw);
        assert.deepEqual(result(() => recovery.parser.parse(source), raw), permissive);
        assert.deepEqual(result(() => stock.parser.configure({dialect: 'json5'}).parse(source), raw), permissive);
        console.log(JSON.stringify({kind: 'parse', name, source, raw: permissive,
            strictRaw: result(() => strict.parser.parse(source), raw),
            editor: result(() => install(json()), byteAt),
            strictEditor: result(() => install(strict), byteAt)}));
    }

    const slot = new Compartment();
    const before = EditorState.create({doc: valid, extensions: slot.of(strict)});
    const original = syntaxTree(before);
    const from = before.doc.length - 1;
    const changes = {from, to: from + 1};
    const without = valid.slice(0, -1);
    const {byteAt} = projectedOffsets(valid);
    const projected = projectedOffsets(without).byteAt;
    const selected = before.update({selection: {anchor: 1}}).state;
    assert.equal(syntaxTree(selected), original);
    const invalid = before.update({changes});
    const rejected = result(() => syntaxTree(invalid.state), projected);
    assert.equal(typeof rejected.offset, 'number');
    assert.equal(syntaxTree(before), original);
    const next = before.update({changes, effects: slot.reconfigure(recovery)}).state;
    assert.notEqual(syntaxTree(next), original);
    assert(stock.isActiveAt(next, 0));
    const swap = next.update({effects: slot.reconfigure(strict)});
    assert.deepEqual(result(() => syntaxTree(swap.state), projected), rejected);
    const repaired = next.update({changes: {from, insert: '}'}, effects: slot.reconfigure(strict)}).state;
    assert.equal(repaired.doc.toString(), before.doc.toString());
    console.log(JSON.stringify({kind: 'edit', source: valid, removeAt: byteAt(from), rejected,
        recovered: rows(syntaxTree(next), projected), repaired: rows(syntaxTree(repaired), byteAt)}));

    // The configured LR stack cut can reject valid JSON. The package linter
    // parses separately, with the host JSON engine rather than the LR tables.
    const depth = 2799;
    const deep = '['.repeat(depth) + '0' + ']'.repeat(depth);
    const failure = result(() => strict.parser.parse(deep), n => n);
    assert.equal(typeof failure.offset, 'number');
    const lint = jsonParseLinter()({state: EditorState.create({doc: deep})});
    assert.deepEqual(lint, []);
    console.log(JSON.stringify({kind: 'depth', depth, strict: failure}));
} finally {
    rmSync(scratch, {recursive: true, force: true});
}
