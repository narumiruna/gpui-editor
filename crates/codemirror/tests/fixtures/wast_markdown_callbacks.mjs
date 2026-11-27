// Run with Node and a directory of local SHA-512-pinned npm archives.
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync, symlinkSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {projectedOffsets} from './editor_line_offsets.mjs';
import {pinnedModules} from './pinned_modules.mjs';

assert.equal(process.argv.length, 3, 'provide the directory of local npm archives');
const root = mkdtempSync(join(realpathSync(tmpdir()), 'wast-markdown-callbacks.'));
const path = process.env.PATH;
try {
    symlinkSync('/usr/bin/tar', join(root, 'tar'));
    process.env.PATH = root; // Do not allow the loader to fetch missing archives.
    const {load} = pinnedModules(resolve(process.argv[2]), root);
    const {Compartment, EditorState} = await load('@codemirror/state');
    const {defineLanguageFacet, foldable, foldNodeProp, sublanguageProp, syntaxTree} =
        await load('@codemirror/language');
    const {markdown, commonmarkLanguage} = await load('@codemirror/lang-markdown');
    const {wastLanguage: stock} = await load('@codemirror/lang-wast');
    const emit = row => console.log(JSON.stringify(row));
    const span = (node, at) => node ? [node.name, at(node.from), at(node.to)] : null;
    const children = (node, at) => {
        const result = [];
        for (let child = node.firstChild; child; child = child.nextSibling) result.push(span(child, at));
        return result;
    };
    // A fence normally joins across a line break. Also force joins exactly at
    // token boundaries using the same LR range parser as the Markdown overlay.
    for (const [source, ranges] of [
        ['(skipfuncSKIP)GAP(0)TAIL', [{from: 0, to: 1}, {from: 5, to: 9}, {from: 13, to: 14}, {from: 17, to: 20}]],
        ['(data "😀"gap)GAP(;Xgap;)HI(func', [{from: 0, to: 10}, {from: 13, to: 14}, {from: 17, to: 20}, {from: 23, to: 25}, {from: 27, to: 32}]],
    ]) {
        const tree = stock.parser.parse(source, [], ranges);
        const {byteAt} = projectedOffsets(source);
        const nodes = [];
        tree.iterate({enter(ref) {
            const node = ref.node;
            nodes.push([span(node, byteAt), span(node.parent, byteAt)]);
        }});
        emit({step: 'LR joins', source, ranges: ranges.map(({from, to}) => [byteAt(from), byteAt(to)]), nodes});
    }

    for (const [step, prefix] of [['quoted', '> '], ['ordinary', '']]) {
        const source = '前 😀\r\n\r\n' + ['```wat', '(module "😀"', '(func 12)', '                          34', ')', '```']
            .map(line => prefix + line).join('\r\n');
        const {byteAt} = projectedOffsets(source);
        const calls = [];
        const snapshot = (node, state) => {
            const {from, to, text} = state.doc.lineAt(node.from);
            return [span(node, byteAt), span(node.parent, byteAt), children(node, byteAt),
                [byteAt(from), byteAt(to), text], byteAt(state.selection.main.head)];
        };
        const app = (node, state) => {
            calls.push(snapshot(node, state));
            return {from: node.firstChild.to + 1, to: node.lastChild.from};
        };
        const facet = defineLanguageFacet({probe: 'number'});
        const configured = stock.configure({props: [
            foldNodeProp.add({App: app}),
            sublanguageProp.add({Module: [{type: 'extend', facet, test(node, state) {
                calls.push(snapshot(node, state));
                return node.name === 'Number' && state.sliceDoc(node.from, node.to) === '12';
            }}]}),
        ]});
        const install = language => markdown({codeLanguages: name => name === 'wat' ? language : null});
        const slot = new Compartment();
        const initial = EditorState.create({doc: source, extensions: [
            slot.of(install(configured)), stock.data.of({probe: 'stock'}),
        ]});
        const text = initial.doc.toString();
        const at = s => text.indexOf(s);
        const first = at('(module'), second = at('(func'), close = text.lastIndexOf(')');
        const state = initial.update({selection: {anchor: at('34')}}).state;
        assert.equal(syntaxTree(state), syntaxTree(initial));
        const control = state.update({effects: slot.reconfigure(install(stock))}).state;
        assert.notEqual(syntaxTree(state), syntaxTree(control));
        const regions = stock.findRegions(state).map(({from, to}) => [byteAt(from), byteAt(to)]);
        const fold = (state, from, to) => {
            const range = foldable(state, from, to);
            return [range ? [byteAt(range.from), byteAt(range.to)] : null, calls.splice(0)];
        };
        const line = state.doc.lineAt(first);
        // Probe both sides of the gap and both fence endpoints. CodeMirror's
        // sublanguage provider resolves with pos - top.from on the mounted node.
        const positions = [...new Set([first, first + 1, second - prefix.length, second - 1, second,
            at('12') + 1, first + at('12'), first + at('12') + 1, first + at('12') + 2, close + 1])]
            .sort((a, b) => a - b);
        const data = positions.map(pos => [byteAt(pos), ...[-1, 1].map(side => [
            stock.isActiveAt(state, pos, side), commonmarkLanguage.isActiveAt(state, pos, side),
            state.languageDataAt('probe', pos, side), calls.splice(0),
        ])]);
        const bounds = [...new Set([line.from, first])].map(from => [from, line.to]);
        if (prefix) bounds.push([line.from, second - 1]);
        const folds = bounds.map(([from, to]) => [byteAt(from), byteAt(to),
            fold(state, from, to), fold(control, from, to)]);
        emit({step, source, regions, data, folds});
    }
} finally {
    process.env.PATH = path;
    rmSync(root, {recursive: true, force: true});
}
