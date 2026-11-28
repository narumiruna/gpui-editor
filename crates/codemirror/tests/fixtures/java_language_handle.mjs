// Run with Node and a directory of local SHA-512-pinned npm archives.
import assert from 'node:assert/strict';
import {existsSync, mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {createRequire} from 'node:module';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {projectedOffsets} from './editor_line_offsets.mjs';
import {pinnedModules} from './pinned_modules.mjs';

assert.equal(process.argv.length, 3, 'provide the directory of local npm archives');
const archives = resolve(process.argv[2]);
const scratch = mkdtempSync(join(realpathSync(tmpdir()), 'java-language-handle.'));
try {
    const {load, pins} = pinnedModules(archives, scratch);
    for (const {name, version} of pins.values()) {
        const file = join(archives, name.replace('/', '__'), `${name.split('/').at(-1)}-${version}.tgz`);
        assert(existsSync(file), `missing offline archive: ${file}`);
    }
    const javaModule = await load('@codemirror/lang-java');
    const {java, javaLanguage: stock} = javaModule;
    const bare = await load('@lezer/java');
    const {NodeProp} = await load('@lezer/common');
    const {Compartment, EditorState, Prec} = await load('@codemirror/state');
    const {defineLanguageFacet, foldNodeProp, indentNodeProp, language, languageDataProp, syntaxTree} = await load('@codemirror/language');
    const catalogModule = await load('@codemirror/language-data');
    const {markdown} = await load('@codemirror/lang-markdown');
    const emit = (kind, fields) => console.log(JSON.stringify({kind, ...fields}));

    const require = createRequire(join(scratch, 'probe.cjs'));
    async function factory(mode, javaModule, bare, catalogModule) {
        const {java, javaLanguage} = javaModule;
        const {languages} = catalogModule;
        const entry = languages.find(x => x.name === 'Java');
        const pending = entry.load();
        assert.equal(entry.load(), pending);
        const catalog = await pending;
        assert.equal(entry.load(), pending);
        const factory = java();
        const exports = Object.keys(javaModule).sort();
        assert.deepEqual(exports, ['java', 'javaLanguage']);
        assert.notEqual(catalog, factory);
        assert.notEqual(factory, java());
        assert.equal(factory.language, javaLanguage);
        assert.equal(catalog.language, stock);
        assert.deepEqual(factory.support, []);
        assert.deepEqual(catalog.support, []);
        assert.notEqual(javaLanguage.parser, bare.parser);
        const sameParser = catalog.language.parser === javaLanguage.parser;
        assert.equal(sameParser, mode === 'ESM');
        emit('factory', {
            mode, exports, sameParser, support: catalog.support,
            entry: {name: entry.name, alias: entry.alias, extensions: entry.extensions},
        });
        return catalog;
    }
    const cat = await factory('ESM', javaModule, bare, catalogModule);
    // The CommonJS catalog calls dynamic import and returns the ESM parser.
    await factory('CJS', require('@codemirror/lang-java'), require('@lezer/java'), require('@codemirror/language-data'));

    const noop = stock.configure({});
    const members = stock.configure({top: 'ClassContent'}, 'Java Members');
    const strict = stock.configure({strict: true}, 'Strict Java');
    const strictMembers = members.configure({strict: true}, 'Strict Members');
    const inherited = strictMembers.configure({}, '');
    const relaxed = inherited.configure({strict: false});
    const switched = relaxed.configure({top: 'Program'});
    const handles = [stock, noop, members, strict, strictMembers, inherited, relaxed, switched];
    assert.equal(new Set(handles.map(h => h.parser)).size, handles.length);
    for (const h of handles) {
        assert.equal(h.data, stock.data);
        assert.equal(h.parser.nodeSet, stock.parser.nodeSet);
    }
    assert.throws(() => stock.configure({top: 'NotJava'}), RangeError);
    assert.deepEqual(stock.parser.topRules, {Program: [0, 3], ClassContent: [1, 209]});
    assert.deepEqual(stock.parser.dialects, {});
    const rows = stock.parser.nodeSet.types.map(t => [
        t.id, t.name, t.isTop, t.isSkipped, t.isError, t.isAnonymous,
        t.prop(NodeProp.group) ?? null, t.prop(NodeProp.openedBy) ?? null,
        t.prop(NodeProp.closedBy) ?? null, t.prop(NodeProp.isolate) ?? null,
        t.prop(languageDataProp) === stock.data,
    ]);
    assert.equal(rows.length, 240);
    emit('types', {names: handles.map(h => h.name), tops: handles.map(h => h.parser.top[1]), rows});
    // Evidence only. The new native handle does not dispatch typed editor callbacks.
    emit('upstream-type-additions', {rows: stock.parser.nodeSet.types.filter(t => t !== bare.parser.nodeSet.types[t.id])
        .map(t => [t.id, !!t.prop(indentNodeProp), !!t.prop(foldNodeProp)])});

    const make = (doc, extensions) => EditorState.create({doc, extensions});
    function result(parse, byteAt) {
        let tree;
        try {
            tree = parse();
        } catch (error) {
            const at = /^No parse at (\d+)$/.exec(error.message);
            if (!(error instanceof SyntaxError) || !at) throw error;
            return {offset: byteAt(Number(at[1]))};
        }
        const rows = [];
        let depth = 0;
        tree.iterate({
            enter(t) { rows.push([depth++, t.type.id, byteAt(t.from), byteAt(t.to)]); },
            leave() { depth--; },
        });
        return {rows};
    }
    const rawByte = source => at => {
        assert(!/^[\uDC00-\uDFFF]/.test(source.slice(at, at + 1)), 'offset bisects surrogate pair');
        return Buffer.byteLength(source.slice(0, at));
    };
    const raw = (source, h) => result(() => h.parser.parse(source), rawByte(source));
    const editor = (source, h) => result(() => syntaxTree(make(source, h)), projectedOffsets(source).byteAt);
    const valid = '// 字😀\r\nclass A {\r  int x = 1;\n}\r\n';
    const cases = [
        ['empty', ''],
        ['members empty', '', members],
        ['mixed valid', valid],
        ['raw hides invalid program', '// 字😀\rclass A { int x = ; }'],
        ['raw hides invalid members', '// 字😀\rint x = ;', members],
        ['members text block', '// 字😀\r\nString s = """\r  字😀\r\n  """;\rint café;', members],
        ['program text block', 'class A {\r\n  String s = """\r  字😀\r\n  """;\r}'],
        ['mid-file syntax', '// 字😀\r\nclass A { int x = @; }\r\n'],
        ['program EOF', '// 字😀\r\nclass A { int x = 1;\r\n'],
        ['members EOF', '// 字😀\r\nint café = 1\r\n', members],
        ['astral error', '// 字😀\r\nclass A { int 😀; }'],
        ['same text as program', 'int x;\r\n'],
        ['same text as class content', 'int x;\r\n', members],
    ];
    for (const h of [stock, members]) for (const depth of [11, 12, 13, 14, 15, 16]) {
        const content = `void m() { ${'A<'.repeat(depth)}B${'>'.repeat(depth)} x; }`;
        const top = h === stock ? 'Program' : 'ClassContent';
        // Depths 12–16 have pinned strict offsets that differ from native.
        cases.push([`adjacent ${depth} ${top}`, h === stock ? `class A { ${content} }` : content, h]);
    }
    for (const [name, source, h = stock] of cases) {
        const top = h === stock ? 'Program' : 'ClassContent';
        const strictHandle = h === stock ? strict : strictMembers;
        const rawStock = raw(source, h), editorStock = editor(source, h);
        const strictRaw = raw(source, strictHandle), strictEditor = editor(source, strictHandle);
        if (h === members) {
            assert.deepEqual(raw(source, relaxed), rawStock);
            assert.deepEqual(editor(source, relaxed), editorStock);
            assert.deepEqual(raw(source, inherited), strictRaw);
            assert.deepEqual(editor(source, inherited), strictEditor);
        }
        emit('parse', {name, source, top, rawStock, editorStock, strictRaw, strictEditor});
    }
    for (const [h, d, top, count] of [[strict, 32, 'Program', 117], [strictMembers, 24, 'ClassContent', 87]]) {
        const content = `void m() { ${'A<'.repeat(d)}B${' >'.repeat(d)} x; }`;
        const source = h === strict ? `class A { ${content} }` : content;
        assert.equal(raw(source, h).rows.length, count);
        // Native returns a work-limit error, not a tree. Record the oracle result separately.
        emit('budget', {top, source, count});
    }

    const shown = '/* 字😀\r\n x */\rint x;\r\nvoid m() {\r\n  run();\r\n}\r\n';
    const byteAt = projectedOffsets(shown).byteAt;
    const alternate = defineLanguageFacet({probe: 'alternate'});
    const marked = members.configure({props: [languageDataProp.add(t => t.name === 'ClassContent' ? alternate : undefined)]});
    const input = [stock.data.of({probe: 'normal'}), Prec.high(members.data.of({probe: 'high'}))];
    const state = make(shown, [...input, members]);
    const markedState = make(shown, [...input, marked]);
    const positions = [0, state.doc.line(3).from, state.doc.length];
    const data = (s, pos) => Object.fromEntries(['probe', 'commentTokens', 'closeBrackets', 'indentOnInput', 'autocomplete']
        .map(key => [key, s.languageDataAt(key, pos).map(value => value instanceof RegExp ? value.source : value)]));
    const active = s => positions.map(p => [-1, 1].map(side => handles.map(h => h.isActiveAt(s, p, side))));
    assert.equal(state.facet(language), members);
    assert.equal(marked.data, stock.data);
    assert.equal(marked.parser.nodeSet.types.filter((type, id) => type !== stock.parser.nodeSet.types[id]).length, 1);
    const triggers = ['case ', '\tcase ', 'case', 'case  ', 'case 1:', 'default:', 'default: ', '{', '}', '\uFEFF{', '\u0085}', '}\n', '\t}\r\n'];
    emit('data', {
        source: shown, positions: positions.map(byteAt), data: positions.map(p => data(state, p)),
        active: active(state), marked: positions.map(p => data(markedState, p)), markedActive: active(markedState),
        regions: stock.findRegions(markedState).map(r => [byteAt(r.from), byteAt(r.to)]),
        triggers: triggers.map(text => [text, state.languageDataAt('indentOnInput', 0)[0].test(text)]),
    });

    const slot = new Compartment();
    const installed = make(valid, slot.of(cat));
    const selected = installed.update({selection: {anchor: 1}}).state;
    assert.equal(syntaxTree(selected), syntaxTree(installed));
    for (const extension of [stock, java()]) {
        const same = selected.update({effects: slot.reconfigure(extension)}).state;
        assert.equal(syntaxTree(same), syntaxTree(installed));
    }
    assert.notEqual(syntaxTree(selected.update({effects: slot.reconfigure(noop)}).state), syntaxTree(installed));
    const before = make(shown, slot.of(strictMembers));
    const at = before.doc.toString().indexOf('x;') + 1;
    const rawAt = byteAt(at);
    const bytes = Buffer.from(shown);
    const after = Buffer.concat([bytes.subarray(0, rawAt), bytes.subarray(rawAt + 1)]).toString();
    const changes = {from: at, to: at + 1};
    const tree = syntaxTree(before);
    const rejected = result(() => syntaxTree(before.update({changes}).state), projectedOffsets(after).byteAt);
    const accepted = before.update({changes, effects: slot.reconfigure(relaxed)}).state;
    const rejectedSwitch = () => syntaxTree(accepted.update({effects: slot.reconfigure(strictMembers)}).state);
    assert.deepEqual(result(rejectedSwitch, projectedOffsets(after).byteAt), rejected);
    const repaired = accepted.update({changes: {from: at, insert: ';'}, effects: slot.reconfigure(strictMembers)}).state;
    assert.equal(syntaxTree(before), tree);
    assert(repaired.doc.eq(before.doc));
    emit('edit', {
        source: shown, at: rawAt, rejected, accepted: result(() => syntaxTree(accepted), projectedOffsets(after).byteAt),
        repaired: result(() => syntaxTree(repaired), byteAt),
        topSwitch: result(() => syntaxTree(repaired.update({effects: slot.reconfigure(stock)}).state), byteAt),
    });

    const splitAt = Buffer.byteLength(valid.slice(0, valid.indexOf('\n')));
    const split = valid.replace('\r\n', '\r// note\n');
    emit('pair', {source: valid, splitAt, joined: editor(valid, stock), split: editor(split, stock)});

    const md = '> ```java\r\n> // 字😀\r\n> class A {}\r\n> ```\r\n';
    const mounted = make(md, markdown({codeLanguages: catalogModule.languages}));
    const regions = stock.findRegions(mounted);
    assert.deepEqual(regions, members.findRegions(mounted));
    emit('markdown', {
        source: md, regions: regions.map(({from, to}) => [from, to].map(projectedOffsets(md).byteAt)),
        presence: regions.map(({from, to}) => [from, to].map(at => [-1, 1].map(side => stock.isActiveAt(mounted, at, side)))),
    });
} finally {
    rmSync(scratch, {recursive: true, force: true});
}
