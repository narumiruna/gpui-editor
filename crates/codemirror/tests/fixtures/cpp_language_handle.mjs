// Run with Node and a directory of local SHA-512-pinned npm archives.
import assert from 'node:assert/strict';
import {existsSync, mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {projectedOffsets} from './editor_line_offsets.mjs';
import {pinnedModules} from './pinned_modules.mjs';

assert.equal(process.argv.length, 3, 'provide the directory of local npm archives');
const archives = resolve(process.argv[2]);
const scratch = mkdtempSync(join(realpathSync(tmpdir()), 'cpp-language-handle.'));
try {
    const {load, pins} = pinnedModules(archives, scratch);
    for (const {name, version} of pins.values()) {
        const file = join(archives, name.replace('/', '__'), `${name.split('/').at(-1)}-${version}.tgz`);
        assert(existsSync(file), `missing offline archive: ${file}`);
    }
    const {cpp, cppLanguage: stock} = await load('@codemirror/lang-cpp');
    const {parser: bare} = await load('@lezer/cpp');
    const {NodeProp} = await load('@lezer/common');
    const {EditorState, Compartment, Prec} = await load('@codemirror/state');
    const {language, defineLanguageFacet, languageDataProp, syntaxTree} = await load('@codemirror/language');
    const {languages} = await load('@codemirror/language-data');
    const {markdown} = await load('@codemirror/lang-markdown');
    const entries = ['C', 'C++'].map(name => languages.find(entry => entry.name === name));
    const jobs = entries.map(entry => entry.load());
    const [c, cc] = await Promise.all(jobs);
    assert.notEqual(jobs[0], jobs[1]);
    assert.notEqual(c, cc);
    assert.notEqual(cpp(), cpp());
    entries.forEach((entry, index) => assert.equal(entry.load(), jobs[index]));
    for (const bundle of [c, cc, cpp()]) {
        assert.equal(bundle.language, stock);
        assert.deepEqual(bundle.support, []);
    }
    assert.notEqual(stock.parser, bare);
    assert.deepEqual(Object.keys(stock.parser.topRules), ['Program']);
    assert.deepEqual(Object.keys(stock.parser.dialects), []);
    const noop = stock.configure({});
    const named = noop.configure({}, 'Named C++');
    const strict = named.configure({strict: true}, 'Strict C++');
    const inherited = strict.configure({}, '');
    const relaxed = inherited.configure({strict: false}, 'Relaxed C++');
    const handles = [stock, noop, named, strict, inherited, relaxed];
    assert.equal(new Set(handles.map(h => h.parser)).size, handles.length);
    for (const h of handles) {
        assert.equal(h.data, stock.data);
        assert.equal(h.parser.nodeSet, stock.parser.nodeSet);
    }
    const emit = (kind, fields) => console.log(JSON.stringify({kind, ...fields}));
    emit('types', {
        names: handles.map(h => h.name),
        rows: stock.parser.nodeSet.types.map(type => [
            type.id, type.name, type.isTop, type.isSkipped, type.isError, type.isAnonymous,
            type.prop(NodeProp.group) ?? null, type.prop(NodeProp.openedBy) ?? null,
            type.prop(NodeProp.closedBy) ?? null, type.prop(NodeProp.isolate) ?? null,
            type.prop(languageDataProp) === stock.data,
        ]),
    });

    function result(run, byteAt) {
        let tree;
        try {
            tree = run();
        } catch (error) {
            const at = /^No parse at (\d+)$/.exec(error.message);
            if (!(error instanceof SyntaxError) || !at) throw error;
            return {offset: byteAt(Number(at[1]))};
        }
        const rows = [];
        let depth = 0;
        tree.iterate({
            enter(node) { rows.push([depth++, node.type.id, byteAt(node.from), byteAt(node.to)]); },
            leave() { depth--; },
        });
        return {rows};
    }
    const rawByte = source => at => {
        assert(!/^[\uDC00-\uDFFF]/.test(source.slice(at, at + 1)), 'offset bisects surrogate pair');
        return Buffer.byteLength(source.slice(0, at));
    };
    const make = (doc, extensions) => EditorState.create({doc, extensions});
    const raw = (source, h) => result(() => h.parser.parse(source), rawByte(source));
    const editor = (source, h) => result(() => syntaxTree(make(source, h)), projectedOffsets(source).byteAt);
    const ambiguous = n => 'T1 a(T2 *b);\n'.repeat(n) + 'T2 e = f<T3>(g);';
    const valid = '// 字😀\r\nint f() {\r  return 5;\r\n}\r\n';
    for (const [name, source] of [
        ['empty', ''],
        ['CRLF after a comment', '// 字😀\r\nint x;\r\n'],
        ['lone CR hides a raw invalid declaration', '// 字😀\rint x = ;'],
        ['ordinary string with CRLF', 'const char *s = "字\r\n😀";\r'],
        ['raw string with mixed breaks', 'const char *s = R"界😀(a\r\nb\rc)界😀";\r\n'],
        ['unexpected token', '// 字😀\r\nconst char *s = "é😀";\rint x = ;\r\n'],
        ['EOF', '// 字😀\r\nint f() {\r\n  return "é😀"\r\n'],
        ['astral unexpected token', '// 字😀\r\nint 😀;'],
        ['nested templates', 'class A { B<C::D<E, F>>::G field; H<I<J>> f() {} };'],
        ['mixed breaks with ambiguity', '// 字😀\r\nT1 a(T2 *b);\r\nT2 e = f<T3>(g);\r'],
        ['pruned GLR', ambiguous(17)],
        ['valid beyond native budget', ambiguous(32)],
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
    let installed = make(valid, slot.of(c));
    const old = syntaxTree(installed);
    for (const extension of [cc, stock, cpp()]) {
        installed = installed.update({effects: slot.reconfigure(extension)}).state;
        assert.equal(syntaxTree(installed), old);
    }
    assert.notEqual(syntaxTree(installed.update({effects: slot.reconfigure(noop)}).state), old);
    const original = make(valid, slot.of(strict));
    const tree = syntaxTree(original);
    assert.equal(syntaxTree(original.update({selection: {anchor: 1}}).state), tree);
    const at = original.doc.toString().indexOf(';');
    const invalid = valid.replace(';', '');
    const cut = {from: at, to: at + 1};
    const read = (transaction, source) => result(() => syntaxTree(transaction().state), projectedOffsets(source).byteAt);
    const rejected = read(() => original.update({changes: cut}), invalid);
    const recovering = original.update({changes: cut, effects: slot.reconfigure(relaxed)}).state;
    assert.equal(recovering.facet(language), relaxed);
    assert.deepEqual(read(() => recovering.update({effects: slot.reconfigure(strict)}), invalid), rejected);
    const swap = insert => recovering.update({changes: {from: at, insert}, effects: slot.reconfigure(strict)});
    const repaired = swap(';').state;
    assert.equal(repaired.facet(language), strict);
    assert(repaired.doc.eq(original.doc));
    assert.equal(syntaxTree(original), tree);
    emit('edit', {
        source: valid, invalid, at: projectedOffsets(valid).byteAt(at), rejected,
        bad: read(() => swap('新'), valid.replace(';', '新')),
        recovered: editor(invalid, relaxed), repaired: editor(valid, strict),
    });

    const data = [stock.data.of({probe: 'normal'}), Prec.high(named.data.of({probe: 'high'}))];
    const repeated = make(valid, [...data, c, cc, cpp(), stock]);
    const distinct = make(valid, [...data, noop, Prec.high(named)]);
    assert.equal(distinct.facet(language), named);
    const probe = state => state.languageDataAt('probe', 0);
    const byteAt = projectedOffsets(valid).byteAt;
    const positions = [0, repeated.doc.line(2).from, repeated.doc.length];
    for (const h of handles) {
        for (const pos of positions) for (const side of [-1, 1]) assert(h.isActiveAt(repeated, pos, side));
    }
    const triggers = ['case ', '\tcase ', 'case', 'case  ', 'case 1:', 'default:', 'default: ', '{', '}', '\uFEFF{', '\u0085}', '}\n'];
    emit('data', {
        source: valid, single: probe(repeated), distinct: probe(distinct), positions: positions.map(byteAt),
        comments: repeated.languageDataAt('commentTokens', 0),
        brackets: repeated.languageDataAt('closeBrackets', 0),
        indent: triggers.map(text => [text, repeated.languageDataAt('indentOnInput', 0)[0].test(text)]),
    });
    const alternate = defineLanguageFacet({probe: 'alternate'});
    const marked = strict.configure({props: [languageDataProp.add(type => type.isTop ? alternate : undefined)]});
    const markedState = make(valid, marked);
    assert(!stock.isActiveAt(markedState, 0));
    assert(!marked.isActiveAt(markedState, 0));
    emit('props', {
        probe: probe(markedState), comments: markedState.languageDataAt('commentTokens', 0),
        regions: marked.findRegions(markedState).map(r => [byteAt(r.from), byteAt(r.to)]),
    });

    const markdownSource = '```c\r\n// 字😀\r\nint a;\r\n```\r\n\r\n```cpp\r\nint b;\r\n```';
    const mounted = make(markdownSource, markdown({codeLanguages: languages}));
    const projected = projectedOffsets(markdownSource).byteAt;
    const regions = stock.findRegions(mounted);
    assert.equal(regions.length, 2);
    assert.deepEqual(named.findRegions(mounted), regions);
    emit('markdown', {
        source: markdownSource,
        regions: regions.map(r => [projected(r.from), projected(r.to)]),
        presence: regions.map(r => [r.from, r.to].map(at => [-1, 1].map(side => stock.isActiveAt(mounted, at, side)))),
    });
} finally {
    rmSync(scratch, {recursive: true, force: true});
}
