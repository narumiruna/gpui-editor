// Run with Node and the directory of SHA-512-pinned npm archives.
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {projectedOffsets} from './editor_line_offsets.mjs';
import {pinnedModules} from './pinned_modules.mjs';

assert.equal(process.argv.length, 3, 'usage: node wast_configured_data.mjs <pinned archive directory>');
const scratch = mkdtempSync(join(realpathSync(tmpdir()), 'gpui-wast-configured-data.'));
try {
    const {load} = pinnedModules(resolve(process.argv[2]), scratch);
    const {Compartment, EditorState, Prec} = await load('@codemirror/state');
    const {defineLanguageFacet, ensureSyntaxTree, languageDataProp, sublanguageProp} = await load('@codemirror/language');
    const {jsonLanguage} = await load('@codemirror/lang-json');
    const {markdown} = await load('@codemirror/lang-markdown');
    const {wastLanguage: stock} = await load('@codemirror/lang-wast');
    const mark = scope => ({scope});
    const root = defineLanguageFacet(mark('root-default'));
    const empty = defineLanguageFacet();
    const absent = null;
    const sub = defineLanguageFacet(mark('sub-default'));
    const replace = defineLanguageFacet();
    const keyword = node => node.name === 'Keyword';
    let calls = 0;
    const rules = sublanguageProp.add({Module: [
        {type: 'extend', facet: sub, test(node, state) {
            calls++;
            return keyword(node) && state.doc.sliceString(node.from, node.to) === 'module' && state.selection.main.head === 0;
        }},
        {type: 'replace', facet: replace, test: keyword},
        {type: 'extend', facet: sub, test(node) {
            assert(!keyword(node), 'the first matching rule must stop the search');
            return false;
        }},
    ]});
    const stockRules = stock.configure({props: [rules]});
    const configured = facet => stockRules.configure({props: [languageDataProp.add(type => type.isTop ? facet : undefined)]});
    const override = configured(root);
    const emptyBase = configured(empty);
    const absentBase = configured(absent);
    for (const lang of [stockRules, override, emptyBase, absentBase]) assert.equal(lang.data, stock.data);

    const data = [
        override.data.of(mark('stock-input')),
        root.of(mark('root-low')),
        Prec.high(root.of(mark('root-high'))),
        sub.of(mark('sub-input')),
        defineLanguageFacet().of(mark('unrelated')),
    ];
    const parser = new Compartment(), extras = new Compartment();
    let source = ';; 😀\r\n(module $m)\r\n(func)\n';
    let state = EditorState.create({doc: source, extensions: [parser.of(stock), extras.of([]), data]});
    let previous;
    function capture(step, expectedCalls) {
        const {units, byteAt} = projectedOffsets(source);
        const tree = ensureSyntaxTree(state, units, 5000);
        assert(tree);
        const input = state.doc.toString();
        const mod = input.indexOf('module'), func = input.indexOf('func');
        const sites = [[0, 1], [mod, -1], [mod, 1], [mod + 6, -1], [mod + 6, 1], [func + 1, 1]];
        const head = byteAt(state.selection.main.head);
        const regions = stock.findRegions(state).map(({from, to}) => [byteAt(from), byteAt(to)]);
        const probes = sites.map(([pos, side]) => {
            const active = override.isActiveAt(state, pos, side);
            assert.equal(active, stock.isActiveAt(state, pos, side));
            return [byteAt(pos), side, active, state.languageDataAt('scope', pos, side),
                state.languageDataAt('closeBrackets', pos, side).map(value => value.brackets)];
        });
        console.log(JSON.stringify({step, source, root: tree.topNode.name, reused: tree === previous, head, regions, probes, calls}));
        assert.equal(calls, expectedCalls * 2, step);
        calls = 0;
        previous = tree;
    }
    function use(languages) {
        state = state.update({effects: parser.reconfigure(languages)}).state;
    }
    capture('stock', 0);
    use(stockRules);
    capture('stock root with rules', 6);
    use(override);
    capture('override', 6);
    use(override);
    capture('same override', 6);
    state = state.update({selection: {anchor: state.doc.toString().indexOf('module') + 1}}).state;
    capture('selection changed', 6);
    state = state.update({effects: extras.reconfigure(replace.of(mark('replace-input')))}).state;
    capture('replace entries changed', 6);
    const retained = state, retainedSource = source;
    const insert = ';; 😀\n';
    state = state.update({changes: {from: 0, insert}, selection: {anchor: 0}}).state;
    source = insert + source;
    capture('edit and select', 6);
    use(emptyBase);
    capture('empty root facet', 6);
    use(absentBase);
    capture('no root facet', 0);
    use([override, jsonLanguage]);
    capture('override before JSON', 12);
    use([Prec.high(jsonLanguage), override]);
    capture('JSON before override', 0);
    use(stock);
    capture('restored stock', 0);
    state = retained;
    source = retainedSource;
    capture('retained state', 6);

    source = '前 😀\n\n```wat\n(module) (func)\n```\n';
    state = EditorState.create({doc: source, extensions: [markdown({codeLanguages: () => stock}), data]});
    capture('stock mount', 0);
} finally {
    rmSync(scratch, {recursive: true, force: true});
}
