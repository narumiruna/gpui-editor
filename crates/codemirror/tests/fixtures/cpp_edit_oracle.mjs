// Generate cpp_edit_oracle.jsonl with Node 22.14+ and the pinned npm archives:
// node crates/codemirror/tests/fixtures/cpp_edit_oracle.mjs <archive-root>
// Use --ambiguous for cpp_ambiguity_oracle.jsonl, --large-recovery for
// cpp_large_recovery.jsonl, or --corpus for the grammar audit.
import assert from 'node:assert/strict';
import {mkdtempSync, readFileSync, readdirSync, realpathSync, rmSync} from 'node:fs';
import {createRequire} from 'node:module';
import {tmpdir} from 'node:os';
import {dirname, join, resolve} from 'node:path';
import {projectedOffsets} from './editor_line_offsets.mjs';
import {pinnedModules} from './pinned_modules.mjs';

const edits = [
    ['c-local-semi', 'int f(void) {\n  int x = 1;\n  return x;\n}\nint tail;\n', '1;', '1'],
    ['c-top-semi', 'int x = 1;\nint f(void) {\n  return x;\n}\n', '1;', '1'],
    ['c-missing-operand-crlf', 'int f(int x) {\r\n  // 🧪\r\n  x += 1;\r\n  return x;\r\n}\r\n', '+= 1;', '+= ;'],
    ['c-missing-condition', 'int f(int x) {\n  if (x) {\n    return 1;\n  } else {\n    return 2;\n  }\n}\n', 'if (x)', 'if ()'],
    ['c-incomplete-call-eof', 'int f(void) {\n  return call(1,\n              2);\n}\n', ');\n}\n', ''],
    ['c-comment-eof-crlf', 'int a;\r\n/* 🧪\r\n   open\r\n */', '*/', ''],
    ['c-stray-scalar-crlf', 'int f(void) {\r\n  \r\n  return 1;\r\n}\r\nint tail;\r\n', '  \r\n', '  🧪\r\n'],
    ['c-missing-close-initializer', 'int values[] = {\n  1, 2\n};\nint tail;\n', '};', ';'],
    ['c-bad-preprocessor', '#include <stdio.h>\nint f(void) {\n  return 1;\n}\n', 'stdio.h>', 'stdio.h'],
    ['c-lone-cr', 'int f(void) {\r  int x = 1;\r  return x;\r}\r', '1;', '1'],
    ['cpp-template-close', 'template<class T> struct Box { T value; };\nBox<Box<int>> item;\nint tail;\n', 'int>>', 'int>'],
    ['cpp-missing-lambda-body', 'auto f(int x) {\n  auto lambda = [x](int y) { return x + y; };\n  return lambda(2);\n}\n', '{ return x + y; }', ''],
    ['cpp-missing-param', 'struct Box {\n  void f(int x, int y);\n  int tail;\n};\n', ' int y', ''],
    ['cpp-unclosed-string-crlf', 'int f() {\r\n  auto s = "🧪";\r\n  return 1;\r\n}\r\n', '🧪"', '🧪'],
    ['cpp-unclosed-raw-eof', 'auto s = u8R"🔑(🧪)🔑";', ')🔑";', ''],
    ['cpp-stray-token', 'namespace xs {\n  int first;\n  int tail;\n}\nint next;\n', 'first;', 'first; =>'],
    ['cpp-missing-class-close', 'struct Record {\n  int x;\n};\n', '};\n', ''],
    ['cpp-missing-switch-colon', 'int f(int x) {\n  switch (x) {\n    case 1:\n      return 2;\n    default:\n      return 3;\n  }\n}\n', 'case 1:', 'case 1'],
    ['cpp-trailing-unfinished-block', 'int f() {\n  return 1;\n    \n}\n', '}\n', ''],
    ['cpp-trailing-unfinished-empty-block', 'int f() {\n  \n}\n', '}\n', ''],
];

const archives = resolve(process.argv[2]);
const mode = process.argv[3];
assert([undefined, '--ambiguous', '--large-recovery', '--corpus'].includes(mode));
const root = realpathSync(mkdtempSync(join(tmpdir(), 'cpp-edit-oracle-')));
try {
    const {install, load} = pinnedModules(archives, root);
    const pkg = await load('@codemirror/lang-cpp');
    const {cpp, cppLanguage} = pkg;
    const exports = ['cpp', 'cppLanguage'];
    assert.deepEqual(Object.keys(pkg), exports);
    const require = createRequire(install('@codemirror/lang-cpp').path);
    assert.deepEqual(Object.keys(require('@codemirror/lang-cpp')).sort(), exports);
    const {parser} = await load('@lezer/cpp');
    const strict = parser.configure({strict: true});
    const {foldable, forceParsing, getIndentation, IndentContext, indentUnit, LanguageSupport, LRLanguage,
           syntaxTree} = await load('@codemirror/language');
    const {EditorState} = await load('@codemirror/state');
    const {highlightTree, tags} = await load('@lezer/highlight');
    const support = cpp();
    assert.ok(support instanceof LanguageSupport);
    assert.notEqual(cpp(), support);
    assert.equal(support.language, cppLanguage);
    assert.ok(cppLanguage instanceof LRLanguage);
    assert.equal(cppLanguage.name, 'cpp');
    for (const extension of [support, cppLanguage]) {
        const state = EditorState.create({doc: 'int a;', extensions: extension});
        const data = key => state.languageDataAt(key, 0);
        assert.deepEqual(data('autocomplete'), []);
        assert.deepEqual(data('commentTokens'), [{line: '//', block: {open: '/*', close: '*/'}}]);
        assert.deepEqual(data('closeBrackets'), [{stringPrefixes: ['L', 'u', 'U', 'u8', 'LR', 'UR', 'uR', 'u8R', 'R']}]);
        const [onInput] = data('indentOnInput');
        assert.deepEqual(['case ', 'default:', '{', '}', 'case', 'case 1:', '}\n'].map(s => onInput.test(s)),
            [true, true, true, true, false, false, false]);
    }
    const names = new Map(Object.entries(tags).filter(([, tag]) => typeof tag !== 'function')
        .map(([name, tag]) => [tag, name]));
    for (const [name, ...modifiers] of [
        ['variableName', 'function', 'definition'], ['variableName', 'function'],
        ['variableName', 'definition'], ['propertyName', 'function'], ['typeName', 'standard'],
        ['string', 'special'], ['name', 'special'],
    ]) {
        names.set(modifiers.reduce((tag, mod) => tags[mod](tag), tags[name]),
            [name, ...modifiers.sort()].join('.'));
    }
    const highlighter = {style(styles) {
        return styles.map(tag => {
            assert.ok(names.has(tag), tag.id);
            return names.get(tag);
        }).join(' ');
    }};
    function complete(state) {
        const view = {state, dispatch(spec) { this.state = this.state.update(spec).state; }};
        assert(forceParsing(view, state.doc.length, 5000));
        return view.state;
    }
    function create(source, language = support) {
        return complete(EditorState.create({doc: source,
            extensions: [language, EditorState.tabSize.of(4), indentUnit.of('    ')]}));
    }
    function snapshot(state, source, id, stage, edit) {
        const {byteAt, units} = projectedOffsets(source);
        assert.equal(units, state.doc.length);
        const syntax = syntaxTree(state);
        const tree = [], spans = [], lines = [], points = [];
        let level = 0;
        syntax.iterate({enter: node => tree.push([level++, node.name, byteAt(node.from), byteAt(node.to)]),
            leave: () => { level--; }});
        highlightTree(syntax, highlighter, (from, to, cls) => spans.push([byteAt(from), byteAt(to), cls]));
        for (let i = 1; i <= state.doc.lines; i++) {
            const {from, to} = state.doc.line(i);
            const fold = foldable(state, from, to);
            lines.push([byteAt(from), byteAt(to), getIndentation(state, from) ?? null,
                fold ? [byteAt(fold.from), byteAt(fold.to)] : null]);
            for (const p of new Set([from, to])) {
                const split = new IndentContext(state, {simulateBreak: p, simulateDoubleBreak: true});
                points.push([byteAt(p), getIndentation(state, p) ?? null, getIndentation(split, p) ?? null]);
            }
        }
        return {id, stage, edit, source, tree, spans, lines, points};
    }
    const emit = row => console.log(JSON.stringify(row));
    function cycle(id, before, removed, inserted, stage, raw, output = emit) {
        const at = before.indexOf(removed);
        assert(at >= 0, id);
        const left = before.slice(0, at), after = left + inserted + before.slice(at + removed.length);
        const normalize = source => source.replace(/\r\n?/g, '\n');
        const makeEdit = (remove, insert) => [Buffer.byteLength(left), Buffer.byteLength(left + remove), insert];
        const update = (state, remove, insert) => complete(state.update({changes: {
            from: normalize(left).length, to: normalize(left + remove).length, insert: normalize(insert),
        }}).state);
        const seed = create(before), edited = update(seed, removed, inserted);
        const restored = update(edited, inserted, removed);
        for (const [step, state, source, edit] of [
            ['before', seed, before, null],
            [stage, edited, after, makeEdit(removed, inserted)],
            ['repaired', restored, before, makeEdit(inserted, removed)],
        ]) {
            const result = snapshot(state, source, id, step, edit);
            for (const language of [support, cppLanguage]) {
                assert.deepEqual(result, snapshot(create(source, language), source, id, step, edit));
            }
            assert.equal(syntaxTree(state).toString(), raw.parse(normalize(source)).toString(), id);
            output(result);
        }
    }
    if (mode === undefined) {
        for (const edit of edits) cycle(...edit, 'broken', parser);
    } else if (mode === '--large-recovery') {
        const limit = 16 * 1024;
        const marker = '{padding}';
        const prefix = '// Editor buffer 🧪\r\n' +
            Array.from({length: 6}, (_, i) => `int prefix${i} = ${i};\r\n`).join('');
        const paddedComment = `/* ${marker} */\r\n`;
        const call = prefix + paddedComment + 'int f(void) {\r\n  return call(1,\r\n              2);\r\n}\r\n';
        for (const [id, source, removed, inserted, bytes] of [
            ['eof-at-limit', call, '              2);\r\n}\r\n', '', limit],
            ['eof-over-limit', call, '              2);\r\n}\r\n', '', limit + 1],
            ['condition-before-large-suffix', prefix + 'int f(int x) {\r  if (x) {\r\n    return 1;\n  }\r\n  return 2;\r\n}\r\n' +
                paddedComment + 'int tail;\r\n', 'if (x)', 'if ()', limit + 1],
            ['invalid-token-after-large-prefix', prefix + paddedComment + 'int f() {\r\n  return 1;\r\n}\r\nint tail;\r\n',
                '  return 1;', '  @🧪\r  return 1;', limit + 32],
        ]) {
            const normalize = text => text.replace(/\r\n?/g, '\n');
            const unpadded = source.replace(marker, '').replace(removed, inserted);
            const pad = bytes - Buffer.byteLength(normalize(unpadded));
            assert(pad > 0);
            const padding = ' '.repeat(pad);
            cycle(id, source.replace(marker, padding), removed, inserted, 'broken', parser, row => {
                if (row.stage === 'broken') assert.equal(Buffer.byteLength(normalize(row.source)), bytes);
                assert(row.source.includes(padding));
                emit({...row, source: row.source.replace(padding, marker), pad});
            });
        }
    } else {
        const dir = join(dirname(install('@lezer/cpp').path), 'test');
        const corpus = new Map();
        for (const file of readdirSync(dir).filter(name => name.endsWith('.txt')).sort()) {
            for (const [, title, source] of readFileSync(join(dir, file), 'utf8')
                .matchAll(/^# ([^\n]*)\n([\s\S]*?)^==>[ \t]*$/gm)) {
                corpus.set(`${file}:${title}`, source);
            }
        }
        assert.equal(corpus.size, 116);
        if (mode === '--corpus') {
            for (const [id, source] of corpus) console.log(JSON.stringify(snapshot(create(source), source, id, 'upstream', null)));
        } else {
            for (const [title, original, simpler] of [
                ['Compound literals without parentheses', '= U<V>{0};', '= (U<V>){0};'],
                ['Nested template calls', '= K::L<M<N>>{};', '= {};'],
                ['Chain of less-than operators', 'd < 0', 'd <= 0'],
            ]) {
                const id = `expressions.txt:${title}`;
                const source = corpus.get(id);
                assert(source?.includes(original), id);
                cycle(id, source.replace(original, simpler), simpler, original, 'edited', strict);
            }
        }
    }
} finally {
    rmSync(root, {recursive: true, force: true});
}
