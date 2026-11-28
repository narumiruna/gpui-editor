// node crates/codemirror/tests/fixtures/vue_custom_blocks.mjs [archive-root]
// Missing archives are downloaded and checked against docs/codemirror-upstream.json.
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {projectedOffsets} from './editor_line_offsets.mjs';
import {pinnedModules} from './pinned_modules.mjs';

const ordinary = '🦀\r\n<template>\r\n  <Widget :label="label" />{{ visible }}\r\n</template>\r\n' +
    '<i18n lang="json" :dict="dict">\r\n{\r\n  "message": "é🦀 {{ ignored }}",\r\n' +
    '  "markup": "<b :value=\'hidden\' /> &amp;"\r\n}\r\n</i18n>\r\n<Widget />';
const raw = '<i18n lang="json" />\n<i18n lang="json"></i18n>\n<i18n lang="text">{{ fallback }}</i18n>\n' +
    '<script type="application/json">{"raw": true}</script>\n' +
    '<script type="module">const x = 1</script>\n' +
    '<script lang="ts">const y: number = 2</script>\n' +
    '<style>.x { color: red; }</style>';
const attributes = '<div>é🦀</div>\r\n<Widget payload=\'{"a": 1}\' :payload=\'{\r\n  "word": "é🦀"\r\n}\' />\r\n' +
    '<Other payload=\'{"no": true}\' />{{ show }}';

const root = mkdtempSync(join(realpathSync(tmpdir()), 'gpui-vue-custom-blocks.'));
try {
    const {load, pins, installed} = pinnedModules(resolve(process.argv[2] ?? root), root);
    const vueExports = await load('@codemirror/lang-vue');
    assert.deepEqual(Object.keys(vueExports), ['vue', 'vueLanguage']);
    const {vue} = vueExports;
    const {html} = await load('@codemirror/lang-html');
    const {jsonLanguage} = await load('@codemirror/lang-json');
    const {javascriptLanguage, typescriptLanguage} = await load('@codemirror/lang-javascript');
    const {cssLanguage} = await load('@codemirror/lang-css');
    const {EditorState} = await load('@codemirror/state');
    const {foldable, getIndentation, indentUnit, syntaxTree} = await load('@codemirror/language');
    const {IterMode, NodeProp} = await load('@lezer/common');
    const {highlightTree, tags} = await load('@lezer/highlight');

    const base = html({nestedLanguages: [
        {tag: 'i18n', attrs: attrs => attrs.lang === 'json', parser: jsonLanguage.parser},
        {tag: 'script', attrs: attrs => attrs.type === 'application/json', parser: jsonLanguage.parser},
        {tag: 'script', attrs: attrs => attrs.type === 'application/json', parser: cssLanguage.parser},
    ], nestedAttributes: [
        {tagName: 'Widget', name: 'payload', parser: jsonLanguage.parser},
        {tagName: 'Widget', name: ':payload', parser: jsonLanguage.parser},
    ]});
    const create = doc => EditorState.create({doc, extensions: [vue({base}), indentUnit.of('  ')]});
    const blockParsers = new Map([
        [jsonLanguage.parser, 'custom-json'], [javascriptLanguage.parser, 'JavaScript'],
        [typescriptLanguage.parser, 'TypeScript'], [cssLanguage.parser, 'Css'],
    ]);
    const tagNames = new Map();
    for (const [name, tag] of Object.entries(tags)) {
        if (typeof tag === 'function') continue;
        tagNames.set(tag, name);
        for (const [modifier, apply] of Object.entries(tags)) {
            if (typeof apply === 'function') tagNames.set(apply(tag), `${name}.${modifier}`);
        }
    }
    const highlighter = {style: list => list.map(tag => {
        assert(tagNames.has(tag));
        return tagNames.get(tag);
    }).join('+') || null};

    function snapshot(state, source, indentLines) {
        const {byteAt, units} = projectedOffsets(source);
        const tree = syntaxTree(state);
        assert.equal(tree.length, units);
        const mounts = [], expressions = [], highlights = [], folds = [];
        function bounds(node, mount) {
            if (!mount.overlay) return [node.from, node.to];
            assert.equal(mount.overlay.length, 1);
            const [{from, to}] = mount.overlay;
            return [node.from + from, node.from + to];
        }
        // Lezer retains the underlying Vue trees. Record only expressions
        // outside body overlays, because those overlays own the highlights.
        tree.iterate({mode: IterMode.IgnoreMounts, enter(node) {
            const mount = node.tree?.prop(NodeProp.mounted);
            if (!mount) return;
            const [from, to] = bounds(node, mount);
            const language = blockParsers.get(mount.parser);
            if (language) {
                mounts.push([byteAt(from), byteAt(to), language, mount.tree.type.name]);
            } else if (!mounts.some(([start, end]) => start <= byteAt(from) && byteAt(to) <= end)) {
                mount.tree.iterate({mode: IterMode.IgnoreMounts, enter(expression) {
                    if (expression.tree?.prop(NodeProp.mounted)?.tree.type.name !== 'SingleExpression') return;
                    expressions.push([byteAt(from + expression.from), byteAt(from + expression.to),
                        expression.name === 'AttributeScript' ? 'Directive' : 'Interpolation']);
                }});
            }
        }});
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
        return {mounts, expressions, highlights, folds, indents};
    }

    console.log(JSON.stringify({capture: 'Vue custom body routes from html({nestedLanguages})',
        packages: [...installed.keys()].sort().map(name => {
            const {version, tarball, integrity} = pins.get(name);
            return {name, version, tarball, integrity};
        })}));
    const state = create(ordinary);
    const from = state.doc.toString().indexOf('lang="json"') + 'lang="'.length;
    const to = from + 'json'.length;
    const {byteAt} = projectedOffsets(ordinary);
    const insert = 'text';
    const updated = ordinary.replace('lang="json"', 'lang="text"');
    console.log(JSON.stringify({name: 'ordinary block and Vue template', source: ordinary,
        ...snapshot(state, ordinary, [7, 9]),
        edit: {from: byteAt(from), to: byteAt(to), insert,
            expected: snapshot(state.update({changes: {from, to, insert}}).state, updated, [])}}));
    console.log(JSON.stringify({name: 'raw override, first match, and stock fallbacks', source: raw,
        ...snapshot(create(raw), raw, [])}));
    const attrState = create(attributes);
    const attrFrom = attrState.doc.toString().indexOf(':payload');
    assert(attrFrom >= 0);
    const attrTo = attrFrom + 1;
    const attrOffsets = projectedOffsets(attributes);
    const attrUpdated = attributes.replace(':payload', 'payload');
    const attrBefore = snapshot(attrState, attributes, [3, 4]);
    const attrAfter = snapshot(attrState.update({changes: {from: attrFrom, to: attrTo}}).state,
        attrUpdated, [3, 4]);
    assert.equal(attrBefore.mounts.length, 1);
    assert.equal(attrAfter.mounts.length, 2);
    assert.equal(attrBefore.expressions.length, 2);
    assert.equal(attrAfter.expressions.length, 1);
    assert.deepEqual(attrAfter, snapshot(create(attrUpdated), attrUpdated, [3, 4]));
    console.log(JSON.stringify({name: 'Vue directive hides an HTML attribute route', source: attributes,
        ...attrBefore, edit: {from: attrOffsets.byteAt(attrFrom), to: attrOffsets.byteAt(attrTo), insert: '',
            expected: attrAfter}}));
} finally {
    rmSync(root, {recursive: true, force: true});
}
