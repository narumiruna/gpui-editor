// node crates/codemirror/tests/fixtures/php_html_routes.mjs [archive-root]
// Missing archives are downloaded and checked against docs/codemirror-upstream.json.
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {projectedOffsets, projectInput} from './editor_line_offsets.mjs';
import {pinnedModules} from './pinned_modules.mjs';

const source = '<?php echo \'🦀\'; ?>\r\n<script type="text/javascript">\r\n' +
    'const x = "é<?= $body ?>🙂";\r\n</script>\r\n';
const root = mkdtempSync(join(realpathSync(tmpdir()), 'gpui-php-html-routes.'));
try {
    const {load, pins} = pinnedModules(resolve(process.argv[2] ?? root), root);
    const {php} = await load('@codemirror/lang-php');
    const {javascriptLanguage} = await load('@codemirror/lang-javascript');
    const {EditorState} = await load('@codemirror/state');
    const {syntaxTree} = await load('@codemirror/language');
    const {IterMode, NodeProp} = await load('@lezer/common');
    const {highlightTree, tags} = await load('@lezer/highlight');
    const create = doc => EditorState.create({doc, extensions: php()});

    function snapshot(state, original) {
        const {byteAt, units} = projectedOffsets(original);
        const tree = syntaxTree(state);
        assert.equal(tree.length, units);
        const outer = tree.prop(NodeProp.mounted);
        assert.equal(outer.tree.type.name, 'Document');
        const origin = outer.overlay[0].from;
        const ranges = outer.overlay.map(({from, to}) => [byteAt(from), byteAt(to)]);
        const mounts = [], strings = [];
        outer.tree.iterate({mode: IterMode.IgnoreMounts, enter(node) {
            const mount = node.tree?.prop(NodeProp.mounted);
            if (!mount) return;
            assert.equal(mount.parser, javascriptLanguage.parser);
            assert.equal(mount.overlay, null);
            const start = byteAt(origin + node.from), end = byteAt(origin + node.to);
            mounts.push(['JavaScript', ranges.map(([from, to]) =>
                [Math.max(start, from), Math.min(end, to)]).filter(([from, to]) => from < to)]);
        }});
        highlightTree(tree, {style: list => list.includes(tags.string) ? 'string' : null},
            (from, to) => strings.push([byteAt(from), byteAt(to)]));
        return {ranges, mounts, strings};
    }

    const state = create(source);
    const from = state.doc.toString().indexOf('javascript') + 'java'.length;
    const insert = '<?= $mode ?>';
    const transaction = state.update({changes: {from, insert}});
    const updated = projectInput(source, transaction);
    const before = snapshot(state, source);
    const after = snapshot(transaction.state, updated);
    assert.deepEqual(after, snapshot(create(updated), updated));
    assert.equal(before.mounts.length, 1);
    assert.equal(after.mounts.length, 0);
    const {byteAt} = projectedOffsets(source);
    const {version, integrity} = pins.get('@codemirror/lang-php');
    console.log(JSON.stringify({version, integrity, source, before,
        edit: [byteAt(from), byteAt(from), insert], after}));
} finally {
    rmSync(root, {recursive: true, force: true});
}
