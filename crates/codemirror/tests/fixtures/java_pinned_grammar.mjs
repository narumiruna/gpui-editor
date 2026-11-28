// Check the Java grammar fixture with Node 22.14+ and the pinned npm archives:
// node crates/codemirror/tests/fixtures/java_pinned_grammar.mjs <archive-root>
// Add --recovery to check the generic recovery edits instead. Add --emit to
// print the selected fixture, or --audit to print the larger residual corpus.
import assert from 'node:assert/strict';
import {mkdtempSync, readFileSync, readdirSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {pathToFileURL} from 'node:url';
import {pinnedModules} from './pinned_modules.mjs';
import {projectedOffsets} from './editor_line_offsets.mjs';

const dir = realpathSync(mkdtempSync(join(tmpdir(), 'java-pinned-grammar-')));
try {
    const {install, load} = pinnedModules(resolve(process.argv[2]), dir);
    const {java, javaLanguage} = await load('@codemirror/lang-java');
    const {parser} = await load('@lezer/java');
    const {NodeProp} = await load('@lezer/common');
    const {highlightTree, tags} = await load('@lezer/highlight');
    const {EditorState} = await load('@codemirror/state');
    const {bracketMatching, foldable, foldNodeProp, forceParsing, getIndentation, IndentContext,
        indentNodeProp, indentUnit, syntaxTree} = await load('@codemirror/language');
    install('@lezer/generator');
    const {fileTests} = await import(pathToFileURL(join(dir, 'node_modules/@lezer/generator/dist/test.js')));

    const support = java();
    assert.equal(javaLanguage.name, 'java');
    assert.equal(support.language, javaLanguage);
    assert.deepEqual(support.support, []);
    const classContent = javaLanguage.configure({top: 'ClassContent'});
    const propertyRows = javaLanguage.parser.nodeSet.types.filter(type => !type.isAnonymous).map(type => [
        type.id, type.name, type.isSkipped, type.prop(NodeProp.group) ?? [],
        type.prop(NodeProp.openedBy) ?? [], type.prop(NodeProp.closedBy) ?? [],
        type.prop(NodeProp.isolate) ?? null, !!type.prop(indentNodeProp), !!type.prop(foldNodeProp),
    ]);
    for (const [id, name, skipped, group, openedBy, closedBy, isolate] of propertyRows) {
        const type = classContent.parser.nodeSet.types[id];
        assert.equal(type, javaLanguage.parser.nodeSet.types[id], name);
        const raw = parser.nodeSet.types[id];
        assert.deepEqual([raw.name, raw.isSkipped, raw.prop(NodeProp.group) ?? [],
            raw.prop(NodeProp.openedBy) ?? [], raw.prop(NodeProp.closedBy) ?? [],
            raw.prop(NodeProp.isolate) ?? null], [name, skipped, group, openedBy, closedBy, isolate]);
    }
    const data = EditorState.create({extensions: support.extension});
    assert.deepEqual(data.languageDataAt('commentTokens', 0), [{line: '//', block: {open: '/*', close: '*/'}}]);
    for (const key of ['autocomplete', 'closeBrackets', 'wordChars']) {
        assert.deepEqual(data.languageDataAt(key, 0), [], key);
    }
    const indentInput = data.languageDataAt('indentOnInput', 0);
    assert.equal(indentInput.length, 1);
    assert.equal(indentInput[0].source, String.raw`^\s*(?:case |default:|\{|\})$`);

    const names = new Map(Object.entries(tags).filter(([, tag]) => typeof tag !== 'function')
        .map(([name, tag]) => [tag, name]));
    for (const [modifier, name] of [['function', 'variableName'], ['definition', 'variableName'], ['standard', 'typeName']]) {
        names.set(tags[modifier](tags[name]), `${name}.${modifier}`);
    }
    const highlighter = {style(styles) {
        return styles.map(tag => {
            assert.ok(names.has(tag));
            return names.get(tag);
        }).join(' ');
    }};
    const settings = [EditorState.tabSize.of(4), indentUnit.of('  ')];
    const audit = process.argv.includes('--audit');
    const recovery = process.argv.includes('--recovery');
    let rendered;
    const matching = recovery ? bracketMatching({renderMatch(hit) {
        assert.equal(rendered, null);
        rendered = hit;
        return [];
    }}) : [];
    const {ViewPlugin} = recovery ? await load('@codemirror/view') : {};
    const plugin = recovery && matching.flat(Infinity).find(extension => extension instanceof ViewPlugin);
    if (recovery) assert(plugin);
    const complete = state => {
        const view = {state, dispatch(spec) { this.state = this.state.update(spec).state; }};
        assert(forceParsing(view, state.doc.length, 5000));
        return view.state;
    };
    function snapshot(id, top, source, allPoints = false, previous, edit) {
        const language = top === 'ClassContent' ? classContent : javaLanguage;
        const raw = language.parser.parse(source);
        let state = EditorState.create({doc: source, extensions: [language, ...settings, matching]});
        if (recovery) {
            state = complete(state);
            if (edit) {
                // These edit inputs are ASCII and use LF, so the byte and editor offsets agree.
                const [from, to, insert] = edit;
                const updated = complete(previous.update({changes: {from, to, insert}}).state);
                assert.equal(updated.doc.toString(), source, id);
                const nodes = tree => {
                    const ranges = [];
                    tree.iterate({enter(node) { ranges.push([node.name, node.from, node.to]); }});
                    return ranges;
                };
                assert.equal(syntaxTree(updated).toString(), syntaxTree(state).toString(), id);
                assert.deepEqual(nodes(syntaxTree(updated)), nodes(syntaxTree(state)), id);
                state = updated;
            }
        }
        assert.equal(syntaxTree(state).length, state.doc.length, id);
        if (top === 'Program') {
            const supported = EditorState.create({doc: source, extensions: [support.extension, ...settings]});
            assert.equal(syntaxTree(supported).toString(), syntaxTree(state).toString(), id);
        }
        const byteAt = unit => Buffer.byteLength(source.slice(0, unit));
        const tree = [], spans = [], lines = [], points = [], positions = new Set();
        let depth = 0;
        raw.iterate({enter: node => tree.push([depth++, node.name, byteAt(node.from), byteAt(node.to)]),
            leave: () => { depth--; }});
        highlightTree(raw, highlighter, (from, to, cls) => spans.push([byteAt(from), byteAt(to), cls]));
        const {byteAt: mapped} = projectedOffsets(source);
        for (let n = 1; n <= state.doc.lines; n++) {
            const {from, to, text} = state.doc.line(n);
            const fold = foldable(state, from, to);
            lines.push([mapped(from), mapped(to), getIndentation(state, from) ?? null,
                fold ? [mapped(fold.from), mapped(fold.to)] : null]);
            positions.add(from).add(to);
            if (allPoints) {
                let at = from;
                for (const ch of text) positions.add(at += ch.length);
            }
        }
        for (const pos of [...positions].sort((a, b) => a - b)) {
            const options = {simulateBreak: pos};
            const indent = opts => getIndentation(new IndentContext(state, opts), pos) ?? null;
            points.push([mapped(pos), getIndentation(state, pos) ?? null, indent(options),
                indent({...options, simulateDoubleBreak: true})]);
        }
        const row = {id, top, source, tree, spans, lines, points};
        if (!recovery) return row;
        if (edit) row.edit = edit;
        const span = ({from, to}) => [mapped(from), mapped(to)];
        row.hits = [...positions].sort((a, b) => a - b).map(anchor => {
            rendered = null;
            plugin.create({state: state.update({selection: {anchor}}).state});
            return [mapped(anchor), rendered && [span(rendered.start), rendered.end ? span(rendered.end) : null, rendered.matched]];
        });
        return [row, state];
    }

    function recoverySnapshots() {
        const rows = [];
        for (const top of ['Program', 'ClassContent']) {
            const program = top === 'Program';
            for (const layout of ['inline', 'multiline']) {
                const multiline = layout === 'multiline';
                const prefix = multiline
                    ? program ? 'class A {\n  void m() {\n    ' : 'void m() {\n  '
                    : program ? 'class A { void m() { ' : 'void m() { ';
                const suffix = multiline
                    ? program ? ' x;\n  }\n}' : ' x;\n}'
                    : program ? ' x; } }' : ' x; }';
                for (const depth of [11, 12, 13]) {
                    const id = `${layout} generics ${depth}`;
                    const start = prefix + 'A<'.repeat(depth) + 'B';
                    const [broken, state] = snapshot(id, top, start + '>'.repeat(depth) + suffix, true);
                    const insert = ' >'.repeat(depth);
                    const [repaired] = snapshot(id, top, start + insert + suffix, true, state,
                        [start.length, start.length + depth, insert]);
                    assert(broken.tree.some(([, name]) => name === '⚠'), id);
                    assert(!repaired.tree.some(([, name]) => name === '⚠'), id);
                    rows.push(broken, repaired);
                }
            }
        }
        return rows;
    }

    const testDir = join(dir, 'node_modules/@lezer/java/test');
    const files = readdirSync(testDir).filter(file => file.endsWith('.txt')).sort();
    const published = [];
    for (const file of files) {
        for (const test of fileTests(readFileSync(join(testDir, file), 'utf8'), file)) {
            test.run(parser);
            published.push(snapshot(`${file}: ${test.name.trim()}`, test.config?.top ?? 'Program', test.text, audit));
        }
    }
    const rows = [...published];
    const add = (id, top, source) => rows.push(snapshot(id, top, source, true));
    add('mixed lines', 'Program', 'class A {\r\n  // 😀\r  void f() {\n    run(\r\n      1);\r  }\n}\r');
    add('mixed class content', 'ClassContent', 'int café;\n// 😀\rA() {\r\n run(1,\r  2);\n}\r');
    add('mixed text block', 'Program', 'class A {\r\n  String s = """\r    😀 text\r\n    """;\n  void f() {}\r}\n');
    add('CRLF comment and delimiters', 'Program', 'class A {\r\n  // 😀\r\n  void f() { run(\r\n    1); }\r\n}\r\n');
    add('block starts after multiline comment', 'Program', '  /* 😀\n     hi */ class T {\n    void f() {}\n}\n');
    add('aligned supplementary character', 'Program', 'class A {\n  void f() { call("😀", other(\n    1,\n    2)); }\n}\n');
    add('annotations, delimiters and groups', 'ClassContent', '@A({1, 2}) int[] xs;\n  @B void f() {\n    g(xs[0], (x) -> x);\n  }\n');
    if (!audit) {
        const generics = n => `${'A<'.repeat(n)}B${' >'.repeat(n)}`;
        for (const n of [10, 16]) {
            const body = `void m() { ${generics(n)} x; }`;
            add(`spaced generics Program ${n}`, 'Program', `class A { ${body} }`);
            add(`spaced generics ClassContent ${n}`, 'ClassContent', body);
        }
        add('spaced generics UTF-8 CRLF Program', 'Program', `class Café {\r\n  // 😀\r\n  void m() { ${generics(12)} café; }\r\n}\r\n`);
        add('spaced generics UTF-8 CRLF ClassContent', 'ClassContent', `// 😀\r\nvoid m() { ${generics(16)} café; }`);
    }
    if (audit) {
        // Bounded single punctuation deletions and invalid-character insertions
        // into the published inputs. Rotate seeds by published row index.
        for (const [index, {id, top, source}] of published.entries()) {
            const punctuation = [...source.matchAll(/[(){}\[\]<>;,=]/g)];
            if (!punctuation.length) continue;
            const drop = punctuation[index % punctuation.length].index;
            const insert = punctuation[(index + 1) % punctuation.length].index;
            add(`delete: ${id}`, top, source.slice(0, drop) + source.slice(drop + 1));
            add(`insert: ${id}`, top, source.slice(0, insert) + '#' + source.slice(insert));
        }
        for (const top of ['Program', 'ClassContent']) {
            for (let n = 1; n <= 16; n++) {
                const wrap = body => top === 'Program' ? `class A { ${body} }` : body;
                add(`adjacent generics ${n}`, top, wrap(`void m() { ${'A<'.repeat(n)}B${'>'.repeat(n)} x; }`));
                add(`spaced generics ${n}`, top, wrap(`void m() { ${'A<'.repeat(n)}B${' >'.repeat(n)} x; }`));
                add(`cast ${n}`, top, wrap(`void m() { x = ${'(T) '.repeat(n)}x; }`));
                add(`partial ${n}`, top, wrap(`void m() { ${'if (x) '.repeat(n)}foo(`));
            }
        }
    }
    const snapshots = recovery ? recoverySnapshots() : [{properties: propertyRows}, ...rows];
    const output = snapshots.map(row => JSON.stringify(row)).join('\n') + '\n';
    if (audit || process.argv.includes('--emit')) {
        process.stdout.write(output);
    } else if (recovery) {
        assert.equal(output, readFileSync(new URL('java_generic_recovery.jsonl', import.meta.url), 'utf8'));
        console.log(`${snapshots.length} pinned Java generic recovery snapshots matched`);
    } else {
        assert.equal(output, readFileSync(new URL('java_pinned_grammar.jsonl', import.meta.url), 'utf8'));
        console.log(`${published.length} published Java grammar cases in ${files.length} files, ${rows.length - published.length} editor cases, ${propertyRows.length} node types matched`);
    }
} finally {
    rmSync(dir, {recursive: true, force: true});
}
