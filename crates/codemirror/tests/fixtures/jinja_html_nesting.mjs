// node crates/codemirror/tests/fixtures/jinja_html_nesting.mjs [archive-root]
// Missing archives are downloaded and checked against docs/codemirror-upstream.json.
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {projectedOffsets} from './editor_line_offsets.mjs';
import {pinnedModules} from './pinned_modules.mjs';

const ordinary = '🦀\r\n<data-block lang="json" data-json=\'{"v":"é{{ a }}?"}\'>\r\n' +
    '{\r\n  "s": "a{{ b }}z"\r\n}\r\n</data-block>\r\n<p data-json=\'{"outside":1}\'></p>';
const raw = '{% raw %}<data-block lang="json">{"raw":"{{ literal }}"}</data-block>{% endraw %}\n' +
    '<script type="application/json">{"n":"{{ name }}"}</script>\n<script>let n = 1</script>\n' +
    '<style lang="c{{ syntax }}ss">p { color: red }</style>\n' +
    '<data-block data-{{ key }}json=\'{"n": 1}\'></data-block>\n' +
    '<data-{{ tag }}block lang="json">true</data-block>';

const root = mkdtempSync(join(realpathSync(tmpdir()), 'gpui-jinja-html-nesting.'));
try {
    const {load, pins, installed} = pinnedModules(resolve(process.argv[2] ?? root), root);
    const {jinja} = await load('@codemirror/lang-jinja');
    const {html} = await load('@codemirror/lang-html');
    const {jsonLanguage} = await load('@codemirror/lang-json');
    const {cssLanguage} = await load('@codemirror/lang-css');
    const {javascriptLanguage} = await load('@codemirror/lang-javascript');
    const {EditorState} = await load('@codemirror/state');
    const {foldable, getIndentation, indentUnit, syntaxTree} = await load('@codemirror/language');
    const {IterMode, NodeProp} = await load('@lezer/common');
    const {highlightTree, tags} = await load('@lezer/highlight');

    const base = html({nestedLanguages: [
        {tag: 'data-block', attrs: attrs => attrs.lang === 'json', parser: jsonLanguage.parser},
        {tag: 'script', attrs: attrs => attrs.type === 'application/json', parser: jsonLanguage.parser},
        {tag: 'script', attrs: attrs => attrs.type === 'application/json', parser: cssLanguage.parser},
    ], nestedAttributes: [{name: 'data-json', tagName: 'data-block', parser: jsonLanguage.parser}]});
    const create = doc => EditorState.create({doc, extensions: [jinja({base}), indentUnit.of('  ')]});
    const parsers = new Map([
        [jsonLanguage.parser, 'custom-json'], [cssLanguage.parser, 'custom-css'],
        [javascriptLanguage.parser, 'JavaScript'],
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

    function snapshot(state, source, indentLines = []) {
        const {byteAt, units} = projectedOffsets(source);
        const tree = syntaxTree(state);
        assert.equal(tree.length, units);
        const outer = tree.prop(NodeProp.mounted);
        assert.equal(outer.parser, base.language.parser);
        const mounts = [], highlights = [], folds = [];
        outer.tree.iterate({mode: IterMode.IgnoreMounts, enter(node) {
            const child = node.tree?.prop(NodeProp.mounted);
            if (!child) return;
            const language = parsers.get(child.parser);
            assert(language, node.name);
            const ranges = child.overlay ?? [{from: 0, to: node.to - node.from}];
            // A child mount can span Jinja gaps, but its input does not include them.
            const parts = ranges.flatMap(({from, to}) => outer.overlay.flatMap(outerRange => {
                const start = Math.max(outerRange.from, outer.overlay[0].from + node.from + from);
                const end = Math.min(outerRange.to, outer.overlay[0].from + node.from + to);
                return start < end ? [[byteAt(start), byteAt(end)]] : [];
            }));
            mounts.push([parts, language, child.tree.type.name]);
        }});
        mounts.sort((a, b) => a[0][0][0] - b[0][0][0]);
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
        const indents = indentLines.map(i => {
            const start = state.doc.line(i).from;
            return [byteAt(start), getIndentation(state, start)];
        });
        return {mounts, highlights, folds, indents};
    }

    console.log(JSON.stringify({capture: 'Jinja outer HTML body and attribute routes',
        packages: [...installed.keys()].sort().map(name => {
            const {version, tarball, integrity} = pins.get(name);
            return {name, version, tarball, integrity};
        })}));
    const state = create(ordinary);
    const from = state.doc.toString().indexOf('lang="json"') + 'lang="'.length;
    const to = from + 'json'.length;
    const {byteAt} = projectedOffsets(ordinary);
    const insert = 'js{{ mode }}on';
    const updated = ordinary.replace('lang="json"', `lang="${insert}"`);
    console.log(JSON.stringify({name: 'projected body and scoped attribute with Jinja gaps', source: ordinary,
        ...snapshot(state, ordinary, [3, 4, 5]),
        edit: {from: byteAt(from), to: byteAt(to), insert,
            expected: snapshot(state.update({changes: {from, to, insert}}).state, updated, [3, 4, 5])}}));
    console.log(JSON.stringify({name: 'raw template text, raw HTML override, and stock fallback', source: raw,
        ...snapshot(create(raw), raw)}));
} finally {
    rmSync(root, {recursive: true, force: true});
}
