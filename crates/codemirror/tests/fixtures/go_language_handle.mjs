// Run with Node and a directory of local SHA-512-pinned npm archives.
import assert from 'node:assert/strict';
import {existsSync, mkdtempSync, readFileSync, readdirSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {dirname, join, resolve} from 'node:path';
import {pathToFileURL} from 'node:url';
import {projectedOffsets} from './editor_line_offsets.mjs';
import {pinnedModules} from './pinned_modules.mjs';

assert.equal(process.argv.length, 3, 'provide the directory of local npm archives');
const archives = resolve(process.argv[2]);
const scratch = mkdtempSync(join(realpathSync(tmpdir()), 'go-language-handle.'));
try {
    const {load, install, pins} = pinnedModules(archives, scratch);
    for (const {name, version} of pins.values()) {
        const file = join(archives, name.replace('/', '__'), `${name.split('/').at(-1)}-${version}.tgz`);
        assert(existsSync(file), `missing offline archive: ${file}`);
    }
    const {go, goLanguage: stock, localCompletionSource} = await load('@codemirror/lang-go');
    const {parser: bare} = await load('@lezer/go');
    const {NodeProp} = await load('@lezer/common');
    const {EditorState, Compartment, Prec} = await load('@codemirror/state');
    const {CompletionContext} = await load('@codemirror/autocomplete');
    const {defineLanguageFacet, languageDataProp, syntaxTree} = await load('@codemirror/language');
    const {languages} = await load('@codemirror/language-data');
    const emit = (kind, fields) => console.log(JSON.stringify({kind, ...fields}));
    const make = (doc, extensions) => EditorState.create({doc, extensions});
    const autocomplete = (state, pos = 0) => state.languageDataAt('autocomplete', pos);
    const entry = languages.find(entry => entry.name === 'Go');
    const loading = entry.load(), catalog = await loading, factory = go(), fresh = go();
    assert.equal(entry.load(), loading);
    assert.notEqual(factory, fresh);
    for (const support of [catalog, factory, fresh]) {
        assert.equal(support.language, stock);
        assert.equal(support.support.length, 2);
    }
    assert.notEqual(stock.parser, bare);
    assert.deepEqual(stock.parser.topRules, {SourceFile: [0, 3]});
    assert.deepEqual(stock.parser.dialects, {});
    assert.notEqual(autocomplete(make('', factory))[0], autocomplete(make('', fresh))[0]);
    assert.equal(autocomplete(make('', factory))[1], localCompletionSource);
    assert.equal(autocomplete(make('', fresh))[1], localCompletionSource);
    emit('factory', {
        name: entry.name, alias: entry.alias, extensions: entry.extensions, filename: entry.filename ?? null,
        repeated: [stock, catalog, [catalog, catalog], [factory, fresh]].map(ext => autocomplete(make('', ext)).length),
    });

    const noop = stock.configure({});
    const strict = noop.configure({strict: true}, 'Strict Go');
    const inherited = strict.configure({}, '');
    const relaxed = inherited.configure({strict: false}, 'Relaxed Go');
    const handles = [stock, noop, strict, inherited, relaxed];
    assert.equal(new Set(handles.map(h => h.parser)).size, handles.length);
    assert(handles.every(h => h.data === stock.data && h.parser.nodeSet === stock.parser.nodeSet));
    emit('types', {
        names: handles.map(h => h.name),
        rows: stock.parser.nodeSet.types.map(type => [
            type.id, type.name, type.isTop, type.isSkipped, type.isError, type.isAnonymous,
            type.prop(NodeProp.group) ?? null, type.prop(NodeProp.openedBy) ?? null,
            type.prop(NodeProp.closedBy) ?? null, type.prop(NodeProp.isolate) ?? null,
            type.prop(languageDataProp) === stock.data,
        ]),
    });

    function result(parse, byteAt) {
        let tree;
        try {
            tree = parse();
        } catch (error) {
            const at = /^No parse at (\d+)$/.exec(error.message);
            if (!(error instanceof SyntaxError) || !at) throw error;
            return {offset: byteAt(Number(at[1]))};
        }
        let depth = 0;
        const rows = [];
        tree.iterate({
            enter(node) { rows.push([depth++, node.type.id, byteAt(node.from), byteAt(node.to)]); },
            leave() { depth--; },
        });
        return {rows};
    }
    const rawByteAt = source => at => {
        assert(!/^[\uDC00-\uDFFF]/.test(source.slice(at, at + 1)), 'offset bisects surrogate pair');
        return Buffer.byteLength(source.slice(0, at));
    };
    const raw = (source, h) => result(() => h.parser.parse(source), rawByteAt(source));
    const editor = (source, h) => result(() => syntaxTree(make(source, h)), projectedOffsets(source).byteAt);
    const source = 'package π\r\n// 😀\rfunc f(arg int) {\n  var café = 1\r\n  var late = arg\r  print(café)\n}\r\n';
    for (const [name, text] of [
        ['empty', ''], ['mixed', source],
        ['lone CR line comment hides invalid input', '// π😀\rvar x = @\n'],
        ['lone CR mid-comment', '// π😀\rvar x = 1\n'],
        ['CRLF line comment', '// π😀\r\nvar x = 1\n'],
        ['lone CR inside block comment', '/* π😀\rnext */\rvar x = 1\n'],
        ['CRLF inside block comment after value', 'var x = 1 /* π😀\r\nnext */ var y = 1\n'],
        ['lone CR inside block comment after value', 'var x = 1 /* π😀\rnext */ var y = 1\n'],
        ['lone CR in interpreted and raw strings', 'var x = "π😀\rtext"\nvar y = `π😀\rtext`\n'],
        ['CRLF in interpreted string', 'var x = "π😀\r\ntext"\n'],
        ['missing brace after mixed UTF8', '// π😀\r\nfunc f() {\r  var 𐐀 = 1\r\n  return 𐐀\r'],
        ['malformed inside a block', 'package p\nfunc f() {\n\tvar = ;\n}\n'],
        ['unfinished block comment', 'package p\n/* 😀\n unfinished'],
        ['bool at EOF', 'var x = true\n'],
        ['nil and line comment semis', 'var x = nil\nvar y = 1 // note\nvar z = 2\n'],
        ['ambiguous generic call', 'func f() { A[T](x) }\n'],
        ['missing selector field', 'package p\nfunc f() {\n var local = 1\n local.\n}\n'],
    ]) {
        emit('parse', {name, source: text, raw: raw(text, stock), editor: editor(text, stock),
            strictRaw: raw(text, strict), strictEditor: editor(text, strict)});
    }

    const generator = install('@lezer/generator');
    const {fileTests} = await import(pathToFileURL(join(dirname(generator.path), 'dist/test.js')));
    const testDir = join(dirname(install('@lezer/go').path), 'test');
    for (const file of readdirSync(testDir).filter(file => file.endsWith('.txt')).sort()) {
        for (const test of fileTests(readFileSync(join(testDir, file), 'utf8'), file)) {
            test.run(bare);
            test.run(stock.parser);
            const permissive = raw(test.text, stock), checked = raw(test.text, strict);
            assert.deepEqual(editor(test.text, stock), permissive);
            assert.deepEqual(editor(test.text, strict), checked);
            assert.deepEqual(checked.rows ?? permissive.rows, permissive.rows);
            emit('package-test', {name: `${file}:${test.name}`, source: test.text,
                rows: permissive.rows, strictError: checked.offset ?? null});
        }
    }

    for (const [name, text] of [
        ['branch', `func f() { ${'A[T]('.repeat(4)}x${')'.repeat(4)} }\n`],
        ['work', `func f() { ${'A['.repeat(8)}T${']'.repeat(8)}(x) }\n`],
    ]) {
        const checked = raw(text, strict);
        assert.deepEqual(editor(text, strict), checked);
        assert(checked.rows.every(row => row[1] !== 0));
        emit('limit', {name, source: text, pinnedRows: checked.rows.length});
    }
    const depth = 2800;
    const deep = `var x = ${'('.repeat(depth)}1${')'.repeat(depth)};\n`;
    emit('depth', {depth, strictRaw: raw(deep, strict), strictEditor: editor(deep, strict)});
    const fallback = 'package p\r\nvar café = 1\r\n' + 'func f(\r\n'.repeat(32);
    emit('fallback', {source: fallback, strictRaw: raw(fallback, strict), strictEditor: editor(fallback, strict),
        rawRows: raw(fallback, stock).rows.length, editorRows: editor(fallback, stock).rows.length});

    const slot = new Compartment();
    let installed = make(source, slot.of(stock));
    const original = syntaxTree(installed);
    for (const ext of [factory, catalog, fresh, stock]) {
        installed = installed.update({effects: slot.reconfigure(ext)}).state;
        assert.equal(syntaxTree(installed), original);
    }
    assert.equal(syntaxTree(installed.update({selection: {anchor: 1}}).state), original);
    assert.notEqual(syntaxTree(installed.update({effects: slot.reconfigure(noop)}).state), original);
    const alternate = defineLanguageFacet({probe: 'alternate'});
    const marked = stock.configure({props: [languageDataProp.add(t => t.isTop ? alternate : undefined)]});
    assert.equal(marked.data, stock.data);
    const data = [stock.data.of({probe: 'normal'}), Prec.high(stock.data.of({probe: 'high'})),
        alternate.of({probe: 'added alternate'})];
    const stockState = make(source, [stock, ...data]), markedState = make(source, [marked, ...data]);
    const mapped = projectedOffsets(source).byteAt;
    assert(!stock.isActiveAt(markedState, 0));
    assert(!marked.isActiveAt(markedState, 0));
    const triggers = ['case', '\tdefault', '}', '\uFEFF}', 'case ', 'case:', 'caseΩ', '}\n', '\u0085}'];
    emit('data', {source, positions: [0, stockState.doc.line(3).from, stockState.doc.length].map(mapped),
        stock: stockState.languageDataAt('probe', 0), marked: markedState.languageDataAt('probe', 0),
        comments: stockState.languageDataAt('commentTokens', 0), brackets: stockState.languageDataAt('closeBrackets', 0),
        markedComments: markedState.languageDataAt('commentTokens', 0),
        regions: marked.findRegions(markedState).map(r => [mapped(r.from), mapped(r.to)]),
        indent: triggers.map(text => [text, stockState.languageDataAt('indentOnInput', 0)[0].test(text)]),
    });

    const valid = '// π😀\r\nfunc f() {\r\n  var café = 1\r\n}\r\n';
    const at = valid.lastIndexOf('}'), invalid = valid.slice(0, at) + valid.slice(at + 1);
    const bad = valid.slice(0, at) + '@' + valid.slice(at + 1);
    emit('edit', {source: valid, at: Buffer.byteLength(valid.slice(0, at)), invalid,
        rejected: editor(invalid, strict), bad: editor(bad, inherited),
        recovered: editor(invalid, relaxed), repaired: editor(valid, strict)});

    const completion = make(source, catalog);
    const fixed = autocomplete(completion)[0];
    const options = fixed(new CompletionContext(make('', catalog), 0, true)).options;
    const text = completion.doc.toString();
    const points = [0, text.indexOf('arg int') + 3, text.lastIndexOf('café') + 4,
        text.indexOf('😀') + 2, text.length];
    const complete = (at, explicit) => autocomplete(completion, at).map((fn, i) => {
        const r = fn(new CompletionContext(completion, at, explicit));
        return r && [mapped(r.from), i ? r.options.map(o => [o.type, o.label]) : r.options.length];
    });
    emit('completion', {source, options: options.map(o => [o.label, o.detail ?? null, o.type, typeof o.apply === 'function']),
        points: points.map(at => [mapped(at), complete(at, false), complete(at, true)]),
    });
} finally {
    rmSync(scratch, {recursive: true, force: true});
}
