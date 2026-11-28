// Verify java_edit_oracle.jsonl with Node 22.14+ and the pinned npm archives:
// node crates/codemirror/tests/fixtures/java_edit_oracle.mjs <archive-root>
// Pass --large to emit the large recovery snapshots.
import assert from 'node:assert/strict';
import {mkdtempSync, readFileSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {pinnedModules} from './pinned_modules.mjs';
import {projectedOffsets} from './editor_line_offsets.mjs';

function largeCases() {
    return [
        ['at-cap', 'Program', 16384, 'class Box {\r\n  int number = 1;\r\n', '}'],
        ['above-cap', 'Program', 16385, 'class Box {\r\n  int number = 1;\r\n', '}'],
        ['class-content', 'ClassContent', 65536, 'int first = 1;\rint value = 2', ';'],
        ['mid-document', 'Program', 131072, 'class Box {\r\n  int value = @;\r\n}', '1', '@'],
    ].flatMap(([id, top, size, tail, insert, replaced]) => {
        const pad = size - Buffer.byteLength('/*é😀*/\r\n' + tail);
        assert(pad >= 0);
        const prefix = `/*é😀${'x'.repeat(pad)}*/\r\n`;
        const source = prefix + tail;
        assert.equal(Buffer.byteLength(source), size);
        const index = replaced == null ? tail.length : tail.indexOf(replaced);
        const repaired = tail.slice(0, index) + insert + tail.slice(index + (replaced?.length ?? 0));
        const start = Buffer.byteLength(prefix + tail.slice(0, index));
        return [
            {id, top, source, pad, tail},
            {id, top, source: prefix + repaired, pad, tail: repaired,
                edit: [start, start + Buffer.byteLength(replaced ?? ''), insert]},
        ];
    });
}

const root = realpathSync(mkdtempSync(join(tmpdir(), 'java-edit-oracle-')));
try {
    const {load} = pinnedModules(resolve(process.argv[2]), root);
    const java = await load('@codemirror/lang-java');
    assert.deepEqual(Object.keys(java), ['java', 'javaLanguage']);
    const support = java.java();
    assert.equal(support.language, java.javaLanguage);
    assert.deepEqual(support.support, []);
    const {foldable, forceParsing, getIndentation, indentUnit, IndentContext, syntaxTree} = await load('@codemirror/language');
    const {EditorState} = await load('@codemirror/state');
    const {highlightTree, tags} = await load('@lezer/highlight');
    const names = new Map(Object.entries(tags).filter(([, tag]) => typeof tag !== 'function')
        .map(([name, tag]) => [tag, name]));
    for (const [modifier, name] of [['function','variableName'], ['definition','variableName'], ['standard','typeName']]) {
        names.set(tags[modifier](tags[name]), `${name}.${modifier}`);
    }
    const highlighter = {style(styles) {
        return styles.map(tag => {
            assert.ok(names.has(tag));
            return names.get(tag);
        }).join(' ');
    }};
    const large = process.argv.includes('--large');
    const rows = large ? largeCases() : readFileSync(new URL('java_edit_oracle.jsonl', import.meta.url), 'utf8')
        .trimEnd().split('\n').map(line => JSON.parse(line));
    let previous;
    for (const row of rows) {
        const {id, top, edit, source, tree, spans, lines, points} = row;
        const language = top === 'Program' ? java.javaLanguage : java.javaLanguage.configure({top});
        const extensions = [language, EditorState.tabSize.of(4), indentUnit.of('  ')];
        const cold = EditorState.create({doc: source, extensions});
        let state = cold;
        if (edit) {
            assert.equal(previous.id, id);
            assert.equal(previous.top, top);
            const [from, to, insert] = edit;
            const before = Buffer.from(previous.source);
            assert.equal(Buffer.concat([before.subarray(0, from), Buffer.from(insert), before.subarray(to)]).toString(), source);
            // Adjacent CR and LF can merge. Compare the editor's normalized
            // documents before constructing the incremental update.
            const a = previous.state.doc.toString(), b = cold.doc.toString();
            let prefix = 0, suffix = 0;
            while (prefix < a.length && a[prefix] === b[prefix]) prefix++;
            while (suffix < a.length - prefix && suffix < b.length - prefix
                   && a.at(-1 - suffix) === b.at(-1 - suffix)) suffix++;
            state = previous.state.update({changes: {from: prefix, to: a.length - suffix,
                insert: b.slice(prefix, b.length - suffix)}}).state;
        }
        const complete = current => {
            const view = {state: current, dispatch(spec) { this.state = this.state.update(spec).state; }};
            assert.ok(forceParsing(view, current.doc.length, 5000));
            return view.state;
        };
        state = complete(state);
        assert.equal(syntaxTree(state).toString(), syntaxTree(complete(cold)).toString(), id);
        previous = {id, top, state, source};

        const raw = language.parser.parse(source);
        const nodes = [], highlights = [], folded = [], indents = [];
        const byteAt = unit => Buffer.byteLength(source.slice(0, unit));
        let level = 0;
        raw.iterate({enter: node => nodes.push([level++, node.name, byteAt(node.from), byteAt(node.to)]),
            leave: () => { level--; }});
        highlightTree(raw, highlighter, (from, to, cls) => highlights.push([byteAt(from), byteAt(to), cls]));
        const {byteAt: mapped} = projectedOffsets(source);
        for (let n = 1; n <= state.doc.lines; n++) {
            const {from, to} = state.doc.line(n);
            const fold = foldable(state, from, to);
            folded.push([mapped(from), mapped(to), getIndentation(state, from) ?? null,
                fold ? [mapped(fold.from), mapped(fold.to)] : null]);
            for (const pos of new Set([from, to])) {
                const split = new IndentContext(state, {simulateBreak: pos, simulateDoubleBreak: true});
                indents.push([mapped(pos), getIndentation(state, pos) ?? null,
                    getIndentation(split, pos) ?? null]);
            }
        }
        if (large) {
            console.log(JSON.stringify({id, top, edit, pad: row.pad, tail: row.tail,
                tree: nodes, spans: highlights, lines: folded, points: indents}));
        } else {
            assert.deepEqual(nodes, tree, `${id} syntax`);
            assert.deepEqual(highlights, spans, `${id} highlights`);
            assert.deepEqual(folded, lines, `${id} folds`);
            assert.deepEqual(indents, points, `${id} indentation`);
        }
    }
    if (!large) console.log(`${rows.length} pinned Java snapshots matched`);
} finally {
    rmSync(root, {recursive: true, force: true});
}
