// Run with Node and a directory of local SHA-512-pinned npm archives.
import assert from 'node:assert/strict';
import {existsSync, mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {projectedOffsets} from './editor_line_offsets.mjs';
import {pinnedModules} from './pinned_modules.mjs';

assert.equal(process.argv.length, 3, 'provide the directory of local npm archives');
const archives = resolve(process.argv[2]);
const scratch = mkdtempSync(join(realpathSync(tmpdir()), 'rust-language-handle.'));
try {
    const {load, pins} = pinnedModules(archives, scratch);
    // Check that the shared loader will not need its network fallback.
    for (const {name, version} of pins.values()) {
        const file = join(archives, name.replace('/', '__'), `${name.split('/').at(-1)}-${version}.tgz`);
        assert(existsSync(file), `missing offline archive: ${file}`);
    }
    const {rust, rustLanguage: stock} = await load('@codemirror/lang-rust');
    const {parser: base} = await load('@lezer/rust');
    const {NodeProp} = await load('@lezer/common');
    const {EditorState, Compartment, Prec} = await load('@codemirror/state');
    const {LRLanguage, language, defineLanguageFacet, languageDataProp, syntaxTree} = await load('@codemirror/language');
    const {languages} = await load('@codemirror/language-data');
    const {json} = await load('@codemirror/lang-json');
    const entry = languages.find(entry => entry.name === 'Rust');
    const loading = entry.load();
    const catalog = await loading;
    assert.equal(entry.load(), loading);
    assert.equal(await entry.load(), catalog);
    assert.notEqual(rust(), rust());
    assert.equal(rust().language, stock);
    assert.equal(catalog.language, stock);
    assert.deepEqual(catalog.support, []);
    assert(stock instanceof LRLanguage);
    assert.notEqual(stock.parser, base);
    assert.equal(base.topNode.prop(languageDataProp), undefined);
    assert.equal(stock.parser.topNode.prop(languageDataProp), stock.data);
    assert.deepEqual(Object.keys(stock.parser.topRules), ['SourceFile']);
    assert.deepEqual(Object.keys(stock.parser.dialects), []);
    assert.throws(() => stock.configure({top: 'Block'}), RangeError);
    const noop = stock.configure({});
    const named = noop.configure({}, 'Named Rust');
    const strict = named.configure({strict: true}, 'Strict Rust');
    const inherited = strict.configure({}, '');
    const relaxed = inherited.configure({strict: false}, 'Relaxed Rust');
    const handles = [stock, noop, named, strict, inherited, relaxed];
    assert.equal(new Set(handles.map(h => h.parser)).size, handles.length);
    for (const h of handles) {
        assert.equal(h.data, stock.data);
        assert.equal(h.parser.topNode, stock.parser.topNode);
        assert.equal(h.parser.nodeSet, stock.parser.nodeSet);
    }
    const emit = (kind, fields) => console.log(JSON.stringify({kind, ...fields}));
    emit('types', {
        names: handles.map(h => h.name),
        types: stock.parser.nodeSet.types.map(type => [
            type.id, type.name, type.isTop, type.isSkipped, type.isError, type.isAnonymous,
            type.prop(NodeProp.group) ?? null, type.prop(NodeProp.openedBy) ?? null,
            type.prop(NodeProp.closedBy) ?? null, type.prop(NodeProp.isolate) ?? null,
        ]),
    });

    function rows(tree, byteAt) {
        const out = [];
        let depth = 0;
        tree.iterate({
            enter(node) { out.push([depth++, node.name, node.type.id, byteAt(node.from), byteAt(node.to)]); },
            leave() { depth--; },
        });
        return out;
    }
    function result(run, byteAt) {
        try {
            return {tree: rows(run(), byteAt)};
        } catch (error) {
            const at = /^No parse at (\d+)$/.exec(error.message);
            if (!(error instanceof SyntaxError) || !at) throw error;
            return {offset: byteAt(Number(at[1]))};
        }
    }
    const rawByte = source => at => {
        assert(!/^[\uDC00-\uDFFF]/.test(source.slice(at, at + 1)), 'offset bisects surrogate pair');
        return Buffer.byteLength(source.slice(0, at));
    };
    const make = (doc, extensions) => EditorState.create({doc, extensions});
    const raw = (source, h) => result(() => h.parser.parse(source), rawByte(source));
    const editor = (source, h) => result(() => syntaxTree(make(source, h)), projectedOffsets(source).byteAt);
    const valid = '// 🦀\r\nfn λ() {\r\n  let s = r"字😀";\r\n}\r\n';
    const invalid = valid.replace(';', '');
    for (const [name, source] of [
        ['empty', ''], ['CRLF', valid], ['lone CR after literal', valid.replace('";\r\n', '";\r')],
        ['lone CR after comment', valid.replace('\r\n', '\r')], ['missing semicolon', invalid],
        ['unexpected token', valid.replace('  let', '  @ let')],
        ['GLR if', 'fn f() { if left && right { 1 } else { 0 } }'],
        ['grammar, not compiler', 'fn f() { missing(); }'], ['unclosed comment', '/* 🦀\r\nnever closed'],
    ]) {
        const rawStock = raw(source, stock), strictRaw = raw(source, strict);
        const editorStock = editor(source, stock), strictEditor = editor(source, strict);
        assert.deepEqual(raw(source, relaxed), rawStock);
        assert.deepEqual(raw(source, inherited), strictRaw);
        assert.deepEqual(editor(source, relaxed), editorStock);
        assert.deepEqual(editor(source, inherited), strictEditor);
        emit('parse', {name, source, raw: rawStock, editor: editorStock, strictRaw, strictEditor});
    }

    const slot = new Compartment();
    const installed = make(valid, slot.of(rust()));
    const old = syntaxTree(installed);
    assert.equal(syntaxTree(installed.update({effects: slot.reconfigure(catalog)}).state), old);
    const newTree = syntaxTree(installed.update({effects: slot.reconfigure(noop)}).state);
    assert.notEqual(newTree, old);
    assert.equal(newTree.type, old.type);
    const original = make(valid, slot.of(strict));
    const tree = syntaxTree(original);
    assert.equal(syntaxTree(original.update({selection: {anchor: 1}}).state), tree);
    const at = original.doc.toString().indexOf(';');
    const cut = {from: at, to: at + 1};
    const reject = (change, source) => result(change, projectedOffsets(source).byteAt);
    const rejected = reject(() => syntaxTree(original.update({changes: cut}).state), invalid);
    const recovering = original.update({changes: cut, effects: slot.reconfigure(relaxed)}).state;
    assert.equal(recovering.facet(language), relaxed);
    assert.deepEqual(reject(() => syntaxTree(recovering.update({effects: slot.reconfigure(strict)}).state), invalid), rejected);
    const swap = insert => recovering.update({changes: {from: at, insert}, effects: slot.reconfigure(strict)}).state;
    const proposed = valid.replace(';', '新');
    const bad = reject(() => syntaxTree(swap('新')), proposed);
    const repaired = swap(';');
    assert.equal(repaired.facet(language), strict);
    assert(repaired.doc.eq(original.doc));
    assert.equal(syntaxTree(original), tree);
    emit('edit', {
        source: valid, invalid, proposed, at: projectedOffsets(valid).byteAt(at), rejected, bad,
        recovered: rows(syntaxTree(recovering), projectedOffsets(invalid).byteAt),
        repaired: rows(syntaxTree(repaired), projectedOffsets(valid).byteAt),
    });

    const probe = state => state.languageDataAt('probe', 0);
    const entries = [stock.data.of({probe: 'normal'}), Prec.high(named.data.of({probe: 'high'}))];
    const repeated = make(valid, [...entries, rust(), stock, catalog, rust()]);
    const distinct = make(valid, [...entries, noop, Prec.high(named)]);
    assert.equal(distinct.facet(language), named);
    const other = make('{}', [strict, Prec.high(json())]);
    assert(!stock.isActiveAt(other, 0));
    assert.deepEqual(stock.findRegions(other), []);
    const byteAt = projectedOffsets(valid).byteAt;
    const positions = [0, repeated.doc.line(2).from, repeated.doc.length];
    for (const h of handles) {
        for (const at of positions) for (const side of [-1, 1]) assert(h.isActiveAt(repeated, at, side));
    }
    const triggers = ['{', '}', '\t}', '\uFEFF{', '\u0085}', '{ ', 'x}', ''];
    emit('data', {
        source: valid, single: probe(repeated), distinct: probe(distinct), positions: positions.map(byteAt),
        regions: stock.findRegions(repeated).map(r => [byteAt(r.from), byteAt(r.to)]),
        comments: repeated.languageDataAt('commentTokens', 0),
        brackets: repeated.languageDataAt('closeBrackets', 0),
        indent: triggers.map(text => [text, repeated.languageDataAt('indentOnInput', 0)[0].test(text)]),
    });

    const mark = new NodeProp();
    const alternate = defineLanguageFacet({probe: 'alternate'});
    const marked = strict.configure({props: [
        mark.add(type => type.isTop ? 0 : type.name === 'Identifier' ? type.id : undefined),
        languageDataProp.add(type => type.isTop ? alternate : undefined),
    ]});
    assert.equal(marked.data, stock.data);
    assert.notEqual(marked.parser.topNode, stock.parser.topNode);
    const markedState = make(valid, marked);
    assert.equal(syntaxTree(markedState).type, marked.parser.topNode);
    assert.equal(marked.parser.parse(valid).type.prop(mark), 0);
    assert(!stock.isActiveAt(markedState, 0));
    assert(!marked.isActiveAt(markedState, 0));
    emit('props', {
        top: marked.parser.topNode.prop(mark), probe: probe(markedState),
        identifiers: marked.parser.nodeSet.types.filter(type => type.name === 'Identifier').map(type => type.prop(mark)),
        regions: marked.findRegions(markedState).map(r => [byteAt(r.from), byteAt(r.to)]),
    });
} finally {
    rmSync(scratch, {recursive: true, force: true});
}
