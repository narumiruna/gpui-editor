// From the repository root, with Node 22.14 or later:
// node crates/codemirror/tests/fixtures/go_oracle.mjs /private/tmp/gpui-upstream-exports.UVI6VVWZ \
//   > crates/codemirror/tests/fixtures/go_oracle.jsonl
// Add --input to emit go_editor_input.jsonl instead.
import assert from 'node:assert/strict';
import {execFileSync} from 'node:child_process';
import {createHash} from 'node:crypto';
import {readFileSync, realpathSync} from 'node:fs';
import {createRequire, findPackageJSON} from 'node:module';
import {dirname, join, resolve} from 'node:path';
import {pathToFileURL} from 'node:url';
import {projectInput, projectedOffsets} from './editor_line_offsets.mjs';

const root = realpathSync(resolve(process.argv[2]));
const manifest = JSON.parse(readFileSync(new URL('../../../../docs/codemirror-upstream.json', import.meta.url)));
const pins = [...manifest.packages, ...manifest.referenceDependencies];
const modules = new Map();
function verify(name, importer) {
    const pin = pins.find(row => row.name === name);
    if (!pin) return; // View also has dependencies outside the reference manifest.
    const dir = join(root, name.replace('/', '__'), 'package');
    assert.equal(realpathSync(findPackageJSON(name, importer)), join(dir, 'package.json'));
    if (modules.has(name)) return;
    const archive = join(dirname(dir), pin.tarball.split('/').at(-1));
    const digest = createHash('sha512').update(readFileSync(archive)).digest('base64');
    assert.equal(`sha512-${digest}`, pin.integrity, name);
    for (const file of ['package.json', 'dist/index.js', 'dist/index.cjs']) {
        assert.deepEqual(readFileSync(join(dir, file)), execFileSync('tar', ['-xOf', archive, `package/${file}`]), `${name}/${file}`);
    }
    const pkg = JSON.parse(readFileSync(join(dir, 'package.json')));
    assert.equal(pkg.name, name);
    assert.equal(pkg.version, pin.version);
    assert.equal(pkg.exports.import ?? pkg.exports['.'].import, './dist/index.js');
    assert.equal(pkg.exports.require ?? pkg.exports['.'].require, './dist/index.cjs');
    const esm = pathToFileURL(join(dir, 'dist/index.js'));
    modules.set(name, esm);
    console.error(`${name} ${pin.version}: verified archive and resolved files`);
    for (const dep of Object.keys(pkg.dependencies ?? {})) verify(dep, esm);
}
const entry = join(root, 'verify-go.mjs');
verify('@codemirror/lang-go', pathToFileURL(entry));
const load = name => import(modules.get(name));
const goModule = await load('@codemirror/lang-go');
const {go, goLanguage, localCompletionSource, snippets} = goModule;
const {EditorSelection, EditorState, Transaction} = await load('@codemirror/state');
const {CompletionContext} = await load('@codemirror/autocomplete');
const {LRLanguage, LanguageSupport, IndentContext, indentUnit, syntaxTree, forceParsing,
       language, getIndentation, foldable, indentOnInput} = await load('@codemirror/language');
const {LRParser} = await load('@lezer/lr');
const {highlightTree, tags, tagHighlighter} = await load('@lezer/highlight');
const exports = ['go', 'goLanguage', 'localCompletionSource', 'snippets'];
assert.deepEqual(Object.keys(goModule), exports);
assert.deepEqual(Object.keys(createRequire(entry)('@codemirror/lang-go')).sort(), exports);
const support = go();
assert.ok(support instanceof LanguageSupport);
assert.notEqual(go(), support);
assert.equal(support.language, goLanguage);
assert.ok(goLanguage instanceof LRLanguage);
assert.ok(goLanguage.parser instanceof LRParser);
assert.equal(goLanguage.name, 'go');
assert.equal(goLanguage.allowsNesting, false);
const tagNames = 'definitionKeyword moduleKeyword controlKeyword keyword bool string character number null variableName variableName.definition typeName labelName propertyName variableName.definition.function typeName.definition variableName.function lineComment blockComment logicOperator arithmeticOperator bitwiseOperator derefOperator updateOperator compareOperator definitionOperator operator modifier separator punctuation paren squareBracket brace'.split(' ');
const highlighter = tagHighlighter(tagNames.map(cls => {
    const [name, ...modifiers] = cls.split('.');
    return {tag: modifiers.reduce((tag, modifier) => tags[modifier](tag), tags[name]), class: cls};
}));
const normalize = raw => raw.replace(/\r\n?/g, '\n');
const byteAt = (raw, pos) => Buffer.byteLength(raw.slice(0, pos));
function offsets(raw) {
    const map = [0];
    let at = 0;
    for (const ch of raw.match(/\r\n|[^]/gu) ?? []) {
        if (ch.length === 2 && ch !== '\r\n') map.push(null);
        map.push(at += Buffer.byteLength(ch));
    }
    return map;
}

// FNV-1a over compact JSON arrays of the complete observations.
function signature(rows) {
    let hash = 2166136261;
    for (const byte of Buffer.from(JSON.stringify(rows))) hash = Math.imul(hash ^ byte, 16777619);
    return [rows.length, hash >>> 0];
}
function complete(state) {
    const view = {state, dispatch(spec) { this.state = this.state.update(spec).state; }};
    assert.ok(forceParsing(view, state.doc.length, 5000));
    assert.equal(syntaxTree(view.state).length, state.doc.length);
    return view.state;
}
const indentPrefixes = ['case', '\tdefault', '}', '\ufeff}', 'case ', 'case:', 'caseΩ', '}\n', '\u0085}'];
const settings = [EditorState.tabSize.of(8), indentUnit.of('    ')];
const extensions = [support, ...settings];
const bare = EditorState.create({doc: 'package π', extensions: goLanguage.extension});
assert.equal(bare.facet(language), goLanguage);
assert.deepEqual(bare.languageDataAt('autocomplete', 0), []);
const catalogState = EditorState.create({extensions});
const [fixedSource, localSource] = catalogState.languageDataAt('autocomplete', 0);
assert.equal(localSource, localCompletionSource);
const fixedCatalog = fixedSource(new CompletionContext(catalogState, 0, true)).options;
assert.equal(snippets.length, 15);
assert.deepEqual(fixedCatalog.slice(0, snippets.length), snippets);
for (const snippet of snippets) assert.equal(typeof snippet.apply, 'function');
assert.deepEqual(fixedCatalog.slice(snippets.length).map(option => option.label),
                 'interface struct chan map package go return break continue goto fallthrough else defer range true false nil'.split(' '));

function snapshot(state, raw) {
    assert.equal(state.doc.toString(), normalize(raw));
    assert.equal(state.facet(language), goLanguage);
    const map = offsets(raw);
    const tree = syntaxTree(state);
    const nodes = [], highlights = [], indents = [], simulated = [], folds = [], fixed = [], local = [];
    let depth = 0, errors = 0;
    tree.iterate({enter: n => {
        nodes.push([depth++, n.name, map[n.from], map[n.to]]);
        if (n.type.isError) errors++;
    }, leave: () => { depth--; }});
    highlightTree(tree, highlighter, (from, to, cls) => highlights.push([map[from], map[to], cls]));
    for (let pos = 0; pos < map.length; pos++) {
        if (map[pos] === null) continue;
        indents.push([map[pos], getIndentation(state, pos)]);
        simulated.push([map[pos], ...[false, true].map(simulateDoubleBreak =>
            getIndentation(new IndentContext(state, {simulateBreak: pos, simulateDoubleBreak}), pos))]);
        for (const explicit of [false, true]) {
            const context = new CompletionContext(state, pos, explicit);
            const keywordResult = fixedSource(context);
            const localResult = localSource(context);
            fixed.push([map[pos], explicit, keywordResult ? map[keywordResult.from] : null]);
            local.push([map[pos], explicit, localResult ? [map[localResult.from], localResult.options.map(o => `${o.type}:${o.label}`)] : null]);
        }
    }
    for (let n = 1; n <= state.doc.lines; n++) {
        const {from, to} = state.doc.line(n);
        const fold = foldable(state, from, to);
        folds.push([map[from], map[to], fold ? [map[fold.from], map[fold.to]] : null]);
    }
    const data = [];
    for (const pos of [0, state.doc.length]) {
        for (const side of [-1, 1]) {
            assert.ok(goLanguage.isActiveAt(state, pos, side));
            const at = key => state.languageDataAt(key, pos, side);
            assert.deepEqual(at('autocomplete'), [fixedSource, localSource]);
            const [onInput] = at('indentOnInput');
            data.push([at('commentTokens').map(m => [m.line, m.block.open, m.block.close]),
                       at('closeBrackets').map(m => m.brackets), at('wordChars'), indentPrefixes.map(s => onInput.test(s))]);
        }
    }
    return {text: signature([raw]), errors, nodes: signature(nodes), highlights: signature(highlights), indents: signature(indents),
            simulated: signature(simulated), folds: signature(folds), fixed: signature(fixed), local: signature(local),
            data: signature(data)};
}
const replace = (raw, old, insert) => {
    const from = raw.indexOf(old);
    assert.ok(from >= 0 && raw.indexOf(old, from + 1) < 0, JSON.stringify(old));
    return [from, from + old.length, insert];
};
function replay(name, raw, steps) {
    let state = complete(EditorState.create({doc: raw, extensions}));
    console.log(JSON.stringify({name, source: raw, expected: snapshot(state, raw)}));
    for (const [name, replacements] of steps) {
        const edits = replacements.map(args => replace(raw, ...args)).sort((a, b) => a[0] - b[0]);
        const changes = edits.map(([from, to, insert]) => [byteAt(raw, from), byteAt(raw, to), insert]);
        const transaction = edits.map(([start, end, insert]) => {
            // Include an adjacent half-break before converting to editor offsets.
            const from = raw[start - 1] === '\r' ? start - 1 : start;
            const to = raw[end] === '\n' ? end + 1 : end;
            const at = normalize(raw.slice(0, from)).length;
            return {from: at, to: at + normalize(raw.slice(from, to)).length,
                    insert: normalize(raw.slice(from, start) + insert + raw.slice(end, to))};
        });
        state = complete(state.update({changes: transaction}).state);
        for (const [from, to, insert] of edits.reverse()) raw = raw.slice(0, from) + insert + raw.slice(to);
        const expected = snapshot(state, raw);
        assert.deepEqual(expected, snapshot(complete(EditorState.create({doc: raw, extensions})), raw), name);
        console.log(JSON.stringify({name, changes, expected}));
    }
}

if (process.argv.includes('--input')) inputIndentation();
else syntaxTrace();

function inputIndentation() {
    for (const [name, marked, typed, event] of [
        ['type a closing brace', 'package p\r\nfunc f() {\r  println("名😀")\r    |\r\n', '}', 'input.type'],
        ['finish the case keyword', 'func f(x int) {\r\n  switch x {\r      cas|\r\n  }\r}\r\n', 'e', 'input.type'],
        ['finish the default keyword', 'func f(x int) {\r\n  switch x {\r      defaul|\r\n  }\r}\r\n', 't', 'input.complete'],
        ['keep indentation on paste', 'package p\r\nfunc f() {\r  println("名😀")\r    |\r\n', '}', 'input.paste'],
    ]) {
        const before = marked.slice(0, marked.indexOf('|'));
        const source = marked.replace('|', '');
        const pos = projectedOffsets(before).units;
        const state = EditorState.create({doc: source, selection: EditorSelection.cursor(pos),
            extensions: [support, indentOnInput()]});
        const {byteAt: originalByte} = projectedOffsets(source);
        const transaction = state.update({...state.replaceSelection(typed), annotations: Transaction.userEvent.of(event)});
        const result = projectInput(source, transaction);
        const finalCaret = projectedOffsets(result).byteAt(transaction.newSelection.main.head);
        console.log(JSON.stringify({name, source, caret: originalByte(pos), typed, event, result, finalCaret}));
    }
}

function syntaxTrace() {
    replay('nested scopes', 'package π\nvar globale = 1\nfunc main(param int) {\n\tvar inner = 2\n\tif param > 0 {\n\t\tvar deep = 3\n\t\tprintln(inner, deep)\n\t}\n}\nvar tail = 4\n', [
        ['unfinished selector inside nested block', [['println(inner, deep)', 'inner.']]],
        ['selector repair and Unicode variable rename', [['inner.', 'println(inner, 深)'], ['var deep', 'var 深']]],
        ['missing initializer before a valid declaration', [['inner = 2', 'inner = ;']]],
        ['stray token after initializer', [['inner = ;', 'inner = 2 @']]],
        ['repair initializer', [['inner = 2 @', 'inner = 2']]],
        ['missing argument and two closers at EOF', [['println(inner, 深)\n\t}\n}\nvar tail = 4\n', 'println(inner,\n']]],
        ['close nested blocks without completing argument', [['println(inner,\n', 'println(inner,\n\t}\n}\n']]],
        ['repair argument and closing delimiters', [['println(inner,\n\t}\n}\n', 'println(inner, 深)\n\t}\n}\n']]],
    ]);

    replay('line breaks and editor services', '// 😀\rpackage p\r\nimport (\r\n\ta "a"\r)\r\nfunc f(x int) {\r\n\tif x != 0 { println(1) }\r\n\tprintln(x,\r\n\t\t0,\r\n\t)\r}\r\n/* hé\r\ncomment */\r\n', [
        ['split a separator and shift a suffix', [['package p\r\n', 'package p\r'], ['comment */', 'comment 🦀 */']]],
        ['close an empty block on its opening line', [['{ println(1) }', '{}']]],
        ['unterminated block comment', [['comment 🦀 */', 'comment 🦀 xx']]],
        ['repair comment at EOF', [['comment 🦀 xx', 'comment */']]],
    ]);

    replay('aligned call with a supplementary letter', 'func f(param int) {\r\n\t𐐀(1,\r\n\t\tparam,\r\n\t)\r\n}\r\n', [
        ['replace the astral letter and keep the alignment', [['𐐀(1', '界(1']]],
    ]);

    replay('aligned call after a decomposed grapheme', 'func f() {\r\n\t_ = "é"; use(1,\r\n\t\t2,\r\n\t)\r\n}\r\n', [
        ['precompose text before the opener', [['é', 'é']]],
    ]);

    const prefix = Array.from({length: 12}, (_, i) => `var prefix${i} = ${i}\r\n`).join('');
    replay('late edit with strict prefix', prefix + 'func f(param int) {\r\n\tvar inner = 2\r\n\treturn inner\r\n}\r\n', [
        ['strict rename', [['var inner', 'var café'], ['return inner', 'return café']]],
        ['late EOF without closing brace', [['return café\r\n}\r\n', 'return café\r\n']]],
        ['repair late EOF', [['return café\r\n', 'return café\r\n}\r\n']]],
        ['strict edit after repair', [['café = 2', 'café = 42']]],
    ]);

    replay('control flow and generic call', 'func f(ch chan int, value any) {\n\tlabel:\n\tfor i := range ch {\n\t\tselect {\n\t\tcase recv := <-ch:\n\t\t\tGo[int](recv)\n\t\tdefault:\n\t\t\tcontinue label\n\t\t}\n\t}\n}\n', [
        ['type switch and case bodies', [['select {\n\t\tcase recv := <-ch:', 'switch recv := value.(type) {\n\t\tcase int:']]],
        ['unfinished if expression in switch case', [['Go[int](recv)', 'if recv >']]],
        ['repair if expression', [['if recv >', 'if recv > i { Go[int](recv) }']]],
    ]);

    replay('recover a bracket inside a styled type name', 'func f(param int) {\n    use({ foo ]  } )\n    var next = 3\n    return next\n}\n', [
        ['repair the malformed conversion', [['use({ foo ]  } )', 'use(param)']]],
        ['skip supplementary characters before a valid declaration', [['use(param)', '😀🦀']]],
        ['repair supplementary token error', [['😀🦀', 'use(param)']]],
    ]);
}
