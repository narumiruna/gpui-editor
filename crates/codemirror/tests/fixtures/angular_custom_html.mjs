// node crates/codemirror/tests/fixtures/angular_custom_html.mjs [archive-root]
// Missing archives are downloaded and checked against docs/codemirror-upstream.json.
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {projectedOffsets, projectInput} from './editor_line_offsets.mjs';
import {pinnedModules} from './pinned_modules.mjs';

const source = [
    '<widget/>{{ dialect }}</widget>',
    `<x-data mode="data" data-json='{"a": "😀"}' [value]="model" (select)="emit(model)">`,
    '  {',
    '  "body": [',
    `    "<b data-json='true'>{{ raw }}</b>"`,
    '  ]',
    '}',
    '</x-data>',
    `<p data-json='"{{ value }}"' onclick="run('{{ arg }}')" [broken="title">{{ outside }}</p>`,
    `<p data-json='false' onclick="run()"></p>`,
    '<script mode="data">{"raw":"{{ script }}"}</script>',
    '<script>let n = 1;</script>',
    '<style>.box { color: red }</style>',
    '<textarea>{{ untouched }}</textarea>',
].join('\r\n');

const root = mkdtempSync(join(realpathSync(tmpdir()), 'gpui-angular-custom-html.'));
try {
    const {load, pins} = pinnedModules(resolve(process.argv[2] ?? root), root);
    const {angular} = await load('@codemirror/lang-angular');
    const {html} = await load('@codemirror/lang-html');
    const {jsonLanguage} = await load('@codemirror/lang-json');
    const {javascriptLanguage} = await load('@codemirror/lang-javascript');
    const {cssLanguage} = await load('@codemirror/lang-css');
    const {EditorState} = await load('@codemirror/state');
    const {foldable, getIndentation, indentUnit, syntaxTree} = await load('@codemirror/language');
    const {IterMode, NodeProp} = await load('@lezer/common');
    const {highlightTree, tags} = await load('@lezer/highlight');
    const json = jsonLanguage.parser;
    const whenData = attrs => attrs.mode === 'data';
    const base = html({selfClosingTags: false, nestedLanguages: [
        {tag: 'x-data', attrs: whenData, parser: json},
        {tag: 'script', attrs: whenData, parser: json},
        {tag: 'script', attrs: whenData, parser: cssLanguage.parser},
    ], nestedAttributes: [
        {name: 'data-json', parser: json},
        {name: '[value]', parser: json},
        {name: '(select)', parser: json},
        {name: '[broken', parser: json},
    ]});
    const create = doc => EditorState.create({doc, extensions: [angular({base}), indentUnit.of('  ')]});
    const parsers = new Map([
        [json, 'json'], [javascriptLanguage.parser, 'JavaScript'], [cssLanguage.parser, 'Css'],
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
    const mounted = node => node.tree?.prop(NodeProp.mounted);

    function snapshot(state, source) {
        const {byteAt, units} = projectedOffsets(source);
        const tree = syntaxTree(state);
        assert.equal(tree.length, units);
        const mounts = [], regions = [], highlights = [], folds = [], indents = [];
        const overlays = [];
        tree.iterate({mode: IterMode.IgnoreMounts, enter(node) {
            if (overlays.some(({from, to}) => from < node.to && node.from < to)) return false;
            const mount = mounted(node);
            if (!mount) return;
            const top = mount.tree.type.name;
            if (top === 'Content' || top === 'Attribute') {
                assert.equal(mount.overlay, null);
                const scripts = [];
                mount.tree.iterate({mode: IterMode.IgnoreMounts, enter(inner) {
                    const script = mounted(inner);
                    if (!script) return;
                    assert.equal(script.overlay, null);
                    scripts.push([script.tree.type.name, byteAt(node.from + inner.from), byteAt(node.from + inner.to)]);
                    return false;
                }});
                regions.push([top, byteAt(node.from), byteAt(node.to), scripts]);
            } else {
                const name = parsers.get(mount.parser);
                assert(name, `unexpected parser ${top}`);
                assert(!mount.overlay || mount.overlay.length === 1);
                const [range] = mount.overlay ?? [{from: 0, to: node.to - node.from}];
                const from = node.from + range.from, to = node.from + range.to;
                if (mount.overlay) overlays.push({from, to});
                mounts.push([name, byteAt(from), byteAt(to), top]);
                return mount.overlay ? undefined : false;
            }
            return false;
        }});
        mounts.sort((a, b) => a[1] - b[1]);
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
            indents.push(getIndentation(state, line.from));
        }
        return {mounts, regions, highlights, folds, indents};
    }

    const state = create(source);
    const input = state.doc.toString();
    const replace = (before, insert) => {
        const from = input.indexOf(before);
        assert(from >= 0);
        return {from, to: from + before.length, insert};
    };
    const changes = [replace('mode="data"', 'mode="html"'), replace('{{ value }}', 'detail')];
    const transaction = state.update({changes});
    const updated = projectInput(source, transaction);
    const {byteAt} = projectedOffsets(source);
    const edits = changes.map(({from, to, insert}) => [byteAt(from), byteAt(to), insert]);
    const before = snapshot(state, source);
    const after = snapshot(transaction.state, updated);
    assert.deepEqual(after, snapshot(create(updated), updated));
    const {version, integrity} = pins.get('@codemirror/lang-angular');
    console.log(JSON.stringify({version, integrity, source, before, edits, after}));
} finally {
    rmSync(root, {recursive: true, force: true});
}
