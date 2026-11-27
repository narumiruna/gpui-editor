// From the repository root, with Node 26.10.0:
// node crates/codemirror/tests/fixtures/json_oracle.mjs <artifact-dir> > crates/codemirror/tests/fixtures/json_repair.jsonl
// node crates/codemirror/tests/fixtures/json_oracle.mjs <artifact-dir> --indent > crates/codemirror/tests/fixtures/json_cr_indent.jsonl
// node crates/codemirror/tests/fixtures/json_oracle.mjs <artifact-dir> --lint-editor > crates/codemirror/tests/fixtures/json_editor_lint.jsonl
// node crates/codemirror/tests/fixtures/json_oracle.mjs <artifact-dir> --indent-input > crates/codemirror/tests/fixtures/json_editor_input.jsonl
import assert from 'node:assert/strict';
import {execFileSync} from 'node:child_process';
import {createHash} from 'node:crypto';
import {readFileSync, realpathSync} from 'node:fs';
import {findPackageJSON} from 'node:module';
import {dirname, join, resolve} from 'node:path';
import {pathToFileURL} from 'node:url';
import {projectedOffsets, projectInput} from './editor_line_offsets.mjs';

const root = resolve(process.argv[2]);
const docs = new URL('../../../../docs/', import.meta.url);
const readDoc = file => JSON.parse(readFileSync(new URL(file, docs)));
const {packages, referenceDependencies} = readDoc('codemirror-upstream.json');
const exported = readDoc('codemirror-upstream-exports.json');
const artifacts = [...exported.packages, ...exported.referenceDependencies];
const modules = new Map();
for (const name of ['@codemirror/lang-json', '@codemirror/language', '@codemirror/state',
                    '@codemirror/view', '@lezer/json', '@lezer/common', '@lezer/lr', '@lezer/highlight']) {
    const pin = [...packages, ...referenceDependencies].find(row => row.name === name);
    const artifact = artifacts.find(row => row.name === name);
    const archive = join(root, name.replace('/', '__'), artifact.artifact);
    const sha512 = `sha512-${createHash('sha512').update(readFileSync(archive)).digest('base64')}`;
    assert.equal(sha512, pin.integrity, name);
    assert.equal(sha512, artifact.verifiedIntegrity, name);
    const dir = realpathSync(join(root, 'node_modules', name));
    assert.equal(dir, join(dirname(archive), 'package'));
    const entry = join(dir, 'dist/index.js');
    for (const file of ['package.json', 'dist/index.js']) {
        assert.deepEqual(readFileSync(join(dir, file)), execFileSync('tar', ['-xOf', archive, `package/${file}`]));
    }
    const pkg = JSON.parse(readFileSync(join(dir, 'package.json')));
    assert.equal(pkg.version, pin.version, name);
    assert.equal(pkg.exports.import ?? pkg.exports['.'].import, './dist/index.js', name);
    modules.set(name, {dir, entry, deps: Object.keys(pkg.dependencies ?? {})});
    console.error(`${name} ${pin.version}: verified archive and resolved files`);
}
for (const {entry, deps} of modules.values()) {
    for (const dep of deps.filter(dep => modules.has(dep))) {
        assert.equal(realpathSync(findPackageJSON(dep, pathToFileURL(entry))), join(modules.get(dep).dir, 'package.json'));
    }
}
const load = name => import(pathToFileURL(modules.get(name).entry));
const {json, jsonParseLinter} = await load('@codemirror/lang-json');
const {EditorSelection, EditorState, Text, Transaction} = await load('@codemirror/state');
const {foldable, foldInside, forceParsing, getIndentation, IndentContext, indentOnInput, syntaxTree} = await load('@codemirror/language');
const {highlightTree, tagHighlighter, tags} = await load('@lezer/highlight');
const {parser} = await load('@lezer/json');
const tagNames = ['string', 'propertyName', 'number', 'bool', 'null', 'separator', 'squareBracket', 'brace'];
const highlighter = tagHighlighter(tagNames.map(name => ({tag: tags[name], class: name})));
const rawText = source => Text.of(source.split('\n'));
const byteAt = (text, pos) => Buffer.byteLength(text.slice(0, pos));
const lint = state => jsonParseLinter()({state}).map(({from, to, severity, message}) =>
    [byteAt(state.doc.toString(), from), byteAt(state.doc.toString(), to), severity, message.toWellFormed()]);

function complete(state) {
    const view = {state, dispatch(spec) { this.state = this.state.update(spec).state; }};
    assert.ok(forceParsing(view, state.doc.length, 5000));
    assert.equal(syntaxTree(view.state).length, state.doc.length);
    return view.state;
}

function snapshot(state) {
    const source = state.doc.toString();
    const byte = pos => byteAt(source, pos);
    const tree = syntaxTree(state);
    assert.equal(tree.toString(), parser.parse(source).toString());
    const nodes = [], nodeFolds = [], highlights = [], folds = [], indents = [];
    let depth = 0;
    tree.iterate({enter: ref => {
        nodes.push([depth++, ref.name, byte(ref.from), byte(ref.to)]);
        if (ref.name === 'Array' || ref.name === 'Object') {
            const fold = foldInside(ref.node);
            if (fold) nodeFolds.push([byte(fold.from), byte(fold.to)]);
        }
    }, leave: () => { depth--; }});
    highlightTree(tree, highlighter, (from, to, tag) => highlights.push([byte(from), byte(to), tag]));
    for (let i = 1; i <= state.doc.lines; i++) {
        const {from, to, text} = state.doc.line(i);
        const fold = foldable(state, from, to - Number(i < state.doc.lines && text.endsWith('\r')));
        folds.push(fold ? [byte(fold.from), byte(fold.to)] : null);
        indents.push(getIndentation(state, from));
    }
    return {source, nodes, nodeFolds, highlights, folds, indents, lint: lint(state)};
}

function repair() {
    const prefix = '[\r\n' + '  {"x":"😀"},\r\n'.repeat(9) + '  ["名😀"';
    const suffix = '],\r\n' + '  [1,true],\r\n'.repeat(8) + '  {}\r\n]\r\n';
    const source = prefix + ',\r\n    false' + suffix;
    let state = complete(EditorState.create({doc: rawText(source), extensions: json()}));
    const steps = [{name: 'initial strict', ...snapshot(state)}];
    for (const [name, from, old, insert] of [
        ['remove separator', prefix.length, ',', ''],
        ['edit malformed input', prefix.length + '\r\n    '.length, 'false', 'null'],
        ['repair separator', prefix.length, '', ','],
    ]) {
        const before = state.doc.toString();
        const to = from + old.length;
        assert.equal(before.slice(from, to), old);
        state = complete(state.update({changes: {from, to, insert: rawText(insert)}}).state);
        const got = snapshot(state);
        assert.deepEqual(got, snapshot(complete(EditorState.create({doc: rawText(got.source), extensions: json()}))));
        steps.push({name, change: [byteAt(before, from), byteAt(before, to), insert], ...got});
    }
    for (const step of steps) console.log(JSON.stringify(step));
}

function indentation() {
    for (const [name, source] of [
        ['lone CR with tab and astral value', '{\r\t"名": [\r  "😀",\r  false\r  ]\r}\r'],
        ['mixed endings and reference lines', ' \t[\r\n   [\r      1\r\n       ], [\r        2\r\r\n      ]\r]\r'],
        ['unfinished containers', '{\r  "unfinished": [\r    10\r'],
        ['missing separator between mixed endings', '[\r   "😀"\r\n  false,\r  null\r]'],
        ['unescaped CR in a string', '[\r  "😀\rcr",\r  false\r]'],
    ]) {
        const state = complete(EditorState.create({doc: source, extensions: json()}));
        const raw = EditorState.create({doc: rawText(source)});
        const indents = [];
        const query = (pos, options) => getIndentation(new IndentContext(state, options), pos);
        const units = source.match(/\r\n|[^]/gu) ?? [];
        assert.equal(state.doc.toString(), source.replace(/\r\n?/g, '\n'));
        let pos = 0, byte = 0;
        for (const unit of ['', ...units]) {
            if (unit) pos += unit === '\r\n' ? 1 : unit.length;
            byte += Buffer.byteLength(unit);
            const lineStart = state.doc.lineAt(pos).from;
            indents.push([byte, query(pos, {}),
                query(pos, {simulateBreak: pos}),
                query(pos, {simulateBreak: pos, simulateDoubleBreak: true}),
                query(pos, {overrideIndentation: from => from === lineStart ? 7 : -1})]);
        }
        assert.equal(pos, state.doc.length);
        console.log(JSON.stringify({name, source, indents, lint: lint(raw), editorLint: lint(state)}));
    }
}

function editorLint() {
    for (const [name, source] of [
        ['missing colon after CRLF and an astral scalar', '\r\n{"名😀" 1}'],
        ['missing separator after mixed endings', '[\r   "😀"\r\n  false,\r  null\r]'],
        ['unterminated string at EOF', '\r\n[\r\n  "名😀'],
        ['leading CRLF before an unexpected token', '\r\n?😀'],
        ['lone CR quoted in the engine message', 'tr\re'],
        ['surrogate split in an excerpt after CRLF', '?a\r\naaaaaaa😀bbbbbbbbbbb'],
        ['valid JSON with mixed endings', '[\r\n"😀",\r\r\ntrue,\rfalse\n]'],
    ]) {
        const state = EditorState.create({doc: source, extensions: json()});
        assert.equal(state.doc.toString(), source.replace(/\r\n?/g, '\n'));
        const {byteAt: mapped, units} = projectedOffsets(source);
        assert.equal(units, state.doc.length);
        const diagnostics = jsonParseLinter()({state}).map(({from, to, severity, message}) =>
            [mapped(from), mapped(to), severity, message.toWellFormed()]);
        console.log(JSON.stringify({name, source, diagnostics}));
    }
}

function inputIndentation() {
    for (const [name, marked, typed, event] of [
        ['object closer on a lone CR line', '{\r\n  "名😀": true\r    |', '}', 'input.type'],
        ['nested array closer', '[\r\n  ["😀"\r    |\r\n  false\r\n]', ']', 'input.type'],
        ['completion of the closer', '{\r\n  "名😀": true\r    |', '}', 'input.complete'],
        ['non-matching input', '{\r\n  "名😀": true\r    |', 'x', 'input.type'],
        ['non-typing event', '{\r\n  "名😀": true\r    |', '}', 'input.paste'],
    ]) {
        const before = marked.slice(0, marked.indexOf('|'));
        const source = marked.replace('|', '');
        const caret = projectedOffsets(before).units;
        const state = EditorState.create({doc: source, selection: EditorSelection.single(caret),
            extensions: [json(), indentOnInput()]});
        const {byteAt: originalByte} = projectedOffsets(source);
        const transaction = state.update({...state.replaceSelection(typed), annotations: Transaction.userEvent.of(event)});
        const result = projectInput(source, transaction);
        const finalCaret = projectedOffsets(result).byteAt(transaction.newSelection.main.head);
        console.log(JSON.stringify({name, source, caret: originalByte(caret), typed, event, result, finalCaret}));
    }
}

if (process.argv.includes('--indent')) indentation();
else if (process.argv.includes('--lint-editor')) editorLint();
else if (process.argv.includes('--indent-input')) inputIndentation();
else repair();
