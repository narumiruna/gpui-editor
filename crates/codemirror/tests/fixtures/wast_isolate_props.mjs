// Run with Node and the directory of SHA-512-pinned npm archives.
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {pinnedModules} from './pinned_modules.mjs';
import {projectedOffsets} from './editor_line_offsets.mjs';

const root = realpathSync(mkdtempSync(join(tmpdir(), 'wast-isolate-props-')));
try {
    const {load} = pinnedModules(resolve(process.argv[2]), root);
    const {wastLanguage} = await load('@codemirror/lang-wast');
    const {NodeProp} = await load('@lezer/common');
    const {EditorState} = await load('@codemirror/state');
    const {Direction, EditorView} = await load('@codemirror/view');
    const {bidiIsolates, forceParsing, syntaxTree} = await load('@codemirror/language');
    const source = '(module (data "ABC אב 🦀"))\r\n;; שלום\r(; آخر ;)\n(data "ASCII")';
    const {byteAt, units} = projectedOffsets(source);
    const location = text => {
        const from = source.indexOf(text);
        assert.ok(from >= 0);
        return [projectedOffsets(source.slice(0, from)).units,
            projectedOffsets(source.slice(0, from + text.length)).units];
    };
    for (const [name, overrides, options = {}] of [
        ['rtl string', {String: 'rtl'}],
        ['ltr comment', {LineComment: 'ltr', BlockComment: 'ltr'}],
        ['suppress string', {String: null}],
        ['nest with app', {App: 'rtl', String: 'ltr'}],
        ['full root', {Module: 'ltr', App: 'rtl', String: 'auto'}],
        ['rtl lines only', {String: 'rtl', BlockComment: null}, {always: false}],
        ['clipped inner', {App: 'rtl', String: 'ltr'}, {visible: [location('ABC אב')]}],
        ['clipped last line', {App: 'ltr'}, {visible: [location('ASCII')]}],
    ]) {
        const language = wastLanguage.configure({props: [NodeProp.isolate.add(type => overrides[type.name])]});
        const {always = true, visible = [[0, units]]} = options;
        const ext = bidiIsolates({alwaysIsolate: always});
        const [plugin] = ext;
        let instance;
        const view = {
            state: EditorState.create({doc: source, extensions: [language, ext]}),
            textDirection: Direction.LTR,
            textDirectionAt: () => Direction.LTR,
            bidiCache: [],
            visibleRanges: visible.map(([from, to]) => ({from, to})),
            plugin: key => key === plugin ? instance : null,
            dispatch(spec) { this.state = this.state.update(spec).state; },
        };
        assert.ok(forceParsing(view, view.state.doc.length, 10000));
        instance = plugin.create(view);
        const [isolates] = view.state.facet(EditorView.bidiIsolatedRanges);
        const [outer] = view.state.facet(EditorView.outerDecorations);
        const decorations = isolates(view);
        assert.equal(decorations, outer(view));
        const marks = [], syntax = [], lines = [];
        for (let mark = decorations.iter(); mark.value; mark.next()) {
            const {class: name, attributes: {dir}, inclusive, bidiIsolate} = mark.value.spec;
            assert.equal(name, 'cm-iso');
            assert.equal(inclusive, true);
            assert.equal(bidiIsolate, dir === 'auto' ? null : dir === 'rtl' ? Direction.RTL : Direction.LTR);
            marks.push([byteAt(mark.from), byteAt(mark.to), dir]);
        }
        syntaxTree(view.state).iterate({enter(node) {
            const dir = node.type.prop(NodeProp.isolate);
            if (dir) syntax.push([node.name, byteAt(node.from), byteAt(node.to), dir]);
        }});
        for (let i = 1; i <= view.state.doc.lines; ++i) {
            const line = view.state.doc.line(i);
            const local = at => byteAt(line.from + at) - byteAt(line.from);
            const spans = EditorView.prototype.bidiSpans.call(view, line);
            lines.push(spans.map(span => [local(span.from), local(span.to), span.level]));
        }
        console.log(JSON.stringify({name, source, overrides, always, visible: visible.map(r => r.map(byteAt)),
            syntax, marks, lines}));
    }
} finally {
    rmSync(root, {recursive: true, force: true});
}
