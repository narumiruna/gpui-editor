// node crates/codemirror/tests/fixtures/liquid_custom_html.mjs [archive-root]
// Missing archives are downloaded and checked against docs/codemirror-upstream.json.
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {projectedOffsets} from './editor_line_offsets.mjs';
import {pinnedModules} from './pinned_modules.mjs';

const source = '😀\r\n<widget/>\r\n<x-data onclick=\'"é{{ attr }}!"\'>\r\n' +
    '  {\r\n    "clé": "hi {{ body }}!"\r\n  }\r\n</x-data>\r\n' +
    '<script mode="data">{"ok": true}</script>\r\n<script>const x = {{ v }}10;</script>\r\n' +
    '{% raw %}<x-data>{"raw": "{{ literal }}"}</x-data>{% endraw %}\r\n' +
    '{% comment %}<x-data>{"hidden": 9}</x-data>{% endcomment %}\r\n' +
    '<script type="text/java{{ dialect }}script">let n = 1;</script>\r\n' +
    '<x-data on{{ event }}click=\'"no"\'>true</x-data>';

const root = mkdtempSync(join(realpathSync(tmpdir()), 'gpui-liquid-custom-html.'));
try {
    const {load, pins} = pinnedModules(resolve(process.argv[2] ?? root), root);
    const {liquid} = await load('@codemirror/lang-liquid');
    const {html} = await load('@codemirror/lang-html');
    const {jsonLanguage} = await load('@codemirror/lang-json');
    const {javascriptLanguage} = await load('@codemirror/lang-javascript');
    const {cssLanguage} = await load('@codemirror/lang-css');
    const {EditorState} = await load('@codemirror/state');
    const {foldable, getIndentation, indentUnit, syntaxTree} = await load('@codemirror/language');
    const {IterMode, NodeProp} = await load('@lezer/common');
    const {highlightTree, tags} = await load('@lezer/highlight');

    const base = html({selfClosingTags: true, nestedLanguages: [
        {tag: 'x-data', parser: jsonLanguage.parser},
        {tag: 'script', attrs: attrs => attrs.mode === 'data', parser: jsonLanguage.parser},
        {tag: 'script', attrs: attrs => attrs.mode === 'data', parser: cssLanguage.parser},
    ], nestedAttributes: [
        {name: 'onclick', tagName: 'x-data', parser: jsonLanguage.parser},
    ]});
    const create = doc => EditorState.create({doc, extensions: [liquid({base}), indentUnit.of('  ')]});
    const parsers = new Map([
        [jsonLanguage.parser, 'json'], [javascriptLanguage.parser, 'JavaScript'],
    ]);
    const names = new Map();
    for (const [name, tag] of Object.entries(tags)) {
        if (typeof tag === 'function') continue;
        names.set(tag, name);
        for (const [modifier, apply] of Object.entries(tags)) {
            if (typeof apply === 'function') names.set(apply(tag), `${name}.${modifier}`);
        }
    }
    const highlighter = {style: list => list.map(tag => {
        assert(names.has(tag));
        return names.get(tag);
    }).join('+') || null};

    function snapshot(state, source) {
        const {byteAt, units} = projectedOffsets(source);
        const tree = syntaxTree(state);
        assert.equal(tree.length, units);
        const outer = tree.prop(NodeProp.mounted);
        assert.equal(outer.parser, base.language.parser);
        const ranges = outer.overlay.map(({from, to}) => [byteAt(from), byteAt(to)]);
        const origin = outer.overlay[0].from;
        const mounts = [], selfClosing = [], highlights = [], folds = [];
        function pieces(from, to) {
            const start = byteAt(origin + from), end = byteAt(origin + to);
            return ranges.map(([a, b]) => [Math.max(start, a), Math.min(end, b)])
                .filter(([a, b]) => a < b);
        }
        outer.tree.iterate({mode: IterMode.IgnoreMounts, enter(node) {
            if (node.name === 'SelfClosingTag') selfClosing.push(pieces(node.from, node.to));
            const mount = node.tree?.prop(NodeProp.mounted);
            if (!mount) return;
            const parser = parsers.get(mount.parser);
            assert(parser, `unexpected parser ${mount.tree.type.name}`);
            const bounds = mount.overlay ?? [{from: 0, to: node.to - node.from}];
            const mapped = bounds.flatMap(({from, to}) => pieces(node.from + from, node.from + to));
            mounts.push([parser, mapped]);
        }});
        mounts.sort((a, b) => a[1][0][0] - b[1][0][0]);
        highlightTree(tree, highlighter, (from, to, tag) => {
            const start = byteAt(from), end = byteAt(to);
            const last = highlights.at(-1);
            if (last?.[1] === start && last[2] === tag) last[1] = end;
            else highlights.push([start, end, tag]);
        });
        for (let i = 1; i <= state.doc.lines; i++) {
            const line = state.doc.line(i);
            const fold = foldable(state, line.from, line.to);
            folds.push(fold ? [byteAt(fold.from), byteAt(fold.to)] : null);
        }
        const indents = [4, 5, 6].map(i => {
            const {from} = state.doc.line(i);
            return [byteAt(from), getIndentation(state, from)];
        });
        return {ranges, mounts, selfClosing, highlights, folds, indents};
    }

    const state = create(source);
    const from = state.doc.toString().indexOf('mode="data"') + 'mode="'.length;
    const to = from + 'data'.length, insert = 'da{{ mode }}ta';
    const updated = source.replace('mode="data"', `mode="${insert}"`);
    const {byteAt} = projectedOffsets(source);
    const before = snapshot(state, source);
    const after = snapshot(state.update({changes: {from, to, insert}}).state, updated);
    assert.deepEqual(after, snapshot(create(updated), updated));
    const {version, integrity} = pins.get('@codemirror/lang-liquid');
    console.log(JSON.stringify({version, integrity, source, before,
        edit: [byteAt(from), byteAt(to), insert], after}));
} finally {
    rmSync(root, {recursive: true, force: true});
}
