// Run: node crates/codemirror/tests/fixtures/wast_style_tags.mjs <pinned archive directory>
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync, symlinkSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {pinnedModules} from './pinned_modules.mjs';

assert.equal(process.argv.length, 3, 'provide the directory of local npm archives');
const root = mkdtempSync(join(realpathSync(tmpdir()), 'gpui-wast-style-tags.'));
const path = process.env.PATH;
try {
    // Permit archive extraction but not downloads. The loader checks SHA-512.
    symlinkSync('/usr/bin/tar', join(root, 'tar'));
    process.env.PATH = root;
    const {load} = pinnedModules(resolve(process.argv[2]), root);
    const {wastLanguage: stock} = await load('@codemirror/lang-wast');
    const {EditorState} = await load('@codemirror/state');
    const {syntaxTree} = await load('@codemirror/language');
    const {styleTags, getStyleTags, highlightTree, tags} = await load('@lezer/highlight');
    const tagNames = new Map(Object.entries(tags).map(([name, tag]) => [tag, name]));
    const rule = source => styleTags(Object.fromEntries(source.map(([selector, names]) =>
        [selector, names.map(name => {assert.ok(tags[name], name); return tags[name];})])));
    for (const invalid of ['/App', 'App/', 'App//Keyword', 'App/*', 'App!!', '""', '"\\q"']) {
        assert.throws(() => rule([[invalid, []]]));
    }
    assert.doesNotThrow(() => rule([['"/" "!" "*" "..."', []]]));

    const selectors = 'nop 42 (module (func $f (i32.const 1)) (block (nop)) (result i32) "éx")';
    const a = [
        ['  App/Keyword  Type ', ['atom']],
        ['Module/*/Keyword', ['bool']],
        ['Module/App/Keyword', ['meta']],
        ['Module/App/App/Keyword', ['controlKeyword']],
        ['Module/Keyword', ['operator']],
        ['App/"\\u0028" App/")"', ['separator']],
        ['App/App/Number', []],
        ['String', []],
    ];
    const b = [
        ['App/Keyword', ['literal']],
        ['Module/App/Keyword', ['definitionKeyword']],
        ['Module/*/Keyword', ['processingInstruction']],
        ['Expression', ['name']],
    ];
    const layers = '  (module  (func (i32.const 1)) (block 2) "éx") \n';
    const c = [
        ['Module/...', ['meta']],
        ['App/...', ['emphasis']],
        ['Module/App', ['strong']],
        ['App/App/App!', ['atom']],
        ['App/Keyword', ['keyword', 'operator']],
        ['String', []],
    ];
    const observations = (tree, source) => {
        const byte = at => Buffer.byteLength(source.slice(0, at));
        const rules = [];
        tree.iterate({enter(node) {
            const r = getStyleTags(node);
            if (r) rules.push([node.name, byte(node.from), byte(node.to), r.tags.map(t => tagNames.get(t)), r.opaque, r.inherit]);
        }});
        const ranges = [[0, source.length], [source.indexOf('const') + 1, source.indexOf('x') + 1], [3, 3]];
        const clips = ranges.map(([from, to]) => {
            const spans = [];
            highlightTree(tree, {style: tags => tags.map(t => tagNames.get(t)).join(' ')},
                (start, end, classes) => spans.push([byte(start), byte(end), classes.split(' ')]), from, to);
            return [byte(from), byte(to), spans];
        });
        return {rules, clips};
    };
    for (const [name, source, sources] of [
        ['within-source', selectors, [a]],
        ['across-sources', selectors, [a, b]],
        ['nested-layers', layers, [c]],
        ['empty-opaque', layers, [c, [['Module/App!', []]]]],
    ]) {
        const lang = stock.configure({props: sources.map(rule)});
        const expected = observations(lang.parser.parse(source), source);
        const state = EditorState.create({doc: source, extensions: lang});
        assert.deepEqual(observations(syntaxTree(state), source), expected);
        console.log(JSON.stringify({name, source, sources, ...expected}));
    }
} finally {
    if (path === undefined) delete process.env.PATH;
    else process.env.PATH = path;
    rmSync(root, {recursive: true, force: true});
}
