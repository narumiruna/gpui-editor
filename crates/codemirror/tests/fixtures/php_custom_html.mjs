// node crates/codemirror/tests/fixtures/php_custom_html.mjs [archive-root]
// Missing archives are downloaded and checked against docs/codemirror-upstream.json.
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {projectedOffsets, projectInput} from './editor_line_offsets.mjs';
import {pinnedModules} from './pinned_modules.mjs';

const source = [
    "<?php echo '🦀'; ?>",
    '<widget/>',
    '<aside></wrong></aside>',
    `<x-data mode="data" onclick='"é<?= $hole ?>🙂"'>`,
    `  "<b data-json='false'>left<?= $hole ?>right</b>"`,
    '</x-data>',
    `<x-<?= $tag ?>data mode="data" onclick='false'>true</x-data>`,
    '<script mode="da<?= $mode ?>ta">const n = "fallback";</script>',
    '<script mode="data">"body"</script>',
    `<p on<?= $attr ?>click='false' onclick="run()"></p>`,
    `<p data-json='true'></p>`,
].join('\r\n');

const root = mkdtempSync(join(realpathSync(tmpdir()), 'gpui-php-custom-html.'));
try {
    const {load, pins} = pinnedModules(resolve(process.argv[2] ?? root), root);
    const {php} = await load('@codemirror/lang-php');
    const {html} = await load('@codemirror/lang-html');
    const {jsonLanguage} = await load('@codemirror/lang-json');
    const {javascriptLanguage} = await load('@codemirror/lang-javascript');
    const {cssLanguage} = await load('@codemirror/lang-css');
    const {EditorState} = await load('@codemirror/state');
    const {syntaxTree} = await load('@codemirror/language');
    const {IterMode, NodeProp} = await load('@lezer/common');
    const {highlightTree, tags} = await load('@lezer/highlight');
    const json = jsonLanguage.parser, css = cssLanguage.parser;
    const whenData = attrs => attrs.mode === 'data';
    const base = html({matchClosingTags: true, selfClosingTags: true, nestedLanguages: [
        {tag: 'x-data', attrs: whenData, parser: json},
        {tag: 'script', attrs: whenData, parser: json},
        {tag: 'script', attrs: whenData, parser: css},
    ], nestedAttributes: [
        {name: 'onclick', tagName: 'x-data', parser: json},
        {name: 'onclick', tagName: 'x-data', parser: css},
        {name: 'data-json', parser: json},
    ]});
    const parsers = new Map([[json, 'json'], [css, 'css'], [javascriptLanguage.parser, 'JavaScript']]);
    const create = (doc, extensions = php({baseLanguage: base.language})) =>
        EditorState.create({doc, extensions});
    const mounted = tree => tree?.prop(NodeProp.mounted);

    function snapshot(state, source) {
        const {byteAt, units} = projectedOffsets(source);
        const tree = syntaxTree(state);
        assert.equal(tree.length, units);
        const outer = mounted(tree);
        assert.equal(outer.tree.type.name, 'Document');
        const dialect = [];
        outer.tree.iterate({mode: IterMode.IgnoreMounts, enter(node) {
            if (node.name === 'SelfClosingTag' || node.name === 'MismatchedCloseTag') dialect.push(node.name);
        }});
        const origin = outer.overlay[0].from;
        const ranges = outer.overlay.map(({from, to}) => [byteAt(from), byteAt(to)]);
        const mounts = [], overlays = [], strings = [];
        outer.tree.iterate({mode: IterMode.IgnoreMounts, enter(node) {
            if (overlays.some(({from, to}) => from < node.to && node.from < to)) return false;
            const mount = mounted(node.tree);
            if (!mount) return;
            const language = parsers.get(mount.parser);
            assert(language);
            const [{from, to}] = mount.overlay ?? [{from: 0, to: node.to - node.from}];
            if (node.name === 'Element') overlays.push({from: node.from + from, to: node.from + to});
            const start = byteAt(origin + node.from + from), end = byteAt(origin + node.from + to);
            const pieces = ranges.map(([from, to]) => [Math.max(start, from), Math.min(end, to)])
                .filter(([from, to]) => from < to);
            mounts.push([language, pieces, mount.tree.type.name]);
        }});
        mounts.sort((a, b) => a[1][0][0] - b[1][0][0]);
        highlightTree(tree, {style: list => list.includes(tags.string) ? 'string' : null},
            (from, to) => strings.push([byteAt(from), byteAt(to)]));
        return {dialect, ranges, mounts, strings};
    }

    const stock = create(source, php());
    let state = create(source), original = source;
    assert.equal(mounted(syntaxTree(state)).parser, base.language.parser);
    const htmlPos = state.doc.toString().indexOf('widget');
    assert.equal(stock.languageDataAt('autocomplete', htmlPos).length, 1);
    assert.equal(state.languageDataAt('autocomplete', htmlPos).length, 0);
    const defaultDialect = snapshot(stock, source).dialect;
    const before = snapshot(state, original);
    assert.deepEqual(defaultDialect, []);
    assert.deepEqual(before.dialect, ['SelfClosingTag', 'MismatchedCloseTag']);
    assert.deepEqual(before.mounts.map(([name]) => name), ['json', 'json', 'JavaScript',
        'JavaScript', 'json', 'JavaScript', 'json']);

    const steps = [];
    function apply(from, to, insert) {
        assert(from >= 0);
        const {byteAt} = projectedOffsets(original);
        const edit = [byteAt(from), byteAt(to), insert];
        const transaction = state.update({changes: {from, to, insert}});
        original = projectInput(original, transaction);
        state = transaction.state;
        const after = snapshot(state, original);
        assert.deepEqual(after, snapshot(create(original), original));
        steps.push({edit, after});
    }
    const mode = state.doc.toString().indexOf('mode="da') + 'mode="da'.length;
    apply(mode, mode, '<?= $route ?>');
    // The mode edit unmasks the body, replacing its JSON route with an inner attribute route.
    assert.equal(steps[0].after.mounts.length, before.mounts.length);
    const [name, [[from, to]]] = steps[0].after.mounts[1];
    assert.equal(name, 'json');
    assert.equal(Buffer.from(original).subarray(from, to).toString(), 'false');
    const old = `<?= $hole ?>🙂"'>\n  `;
    const mixed = state.doc.toString().indexOf(old);
    apply(mixed, mixed + old.length, `<?= '🦀' ?>中"'> `);
    const {version, integrity} = pins.get('@codemirror/lang-php');
    console.log(JSON.stringify({version, integrity, source, defaultDialect, before, steps}));
} finally {
    rmSync(root, {recursive: true, force: true});
}
