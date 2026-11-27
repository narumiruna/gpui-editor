// Run with Node and a directory of local SHA-512-pinned npm archives.
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {projectedOffsets, projectInput} from './editor_line_offsets.mjs';
import {pinnedModules} from './pinned_modules.mjs';

assert.equal(process.argv.length, 3, 'provide the directory of local npm archives');
const scratch = mkdtempSync(join(realpathSync(tmpdir()), 'lr-highlight-scopes.'));
try {
    const {load} = pinnedModules(resolve(process.argv[2]), scratch);
    const {EditorState, Compartment} = await load('@codemirror/state');
    const {HighlightStyle, languageDataProp, syntaxTree} = await load('@codemirror/language');
    const {highlightTree, tags} = await load('@lezer/highlight');
    const {NodeProp} = await load('@lezer/common');
    const {markdown} = await load('@codemirror/lang-markdown');
    const {jsonLanguage: json} = await load('@codemirror/lang-json');
    const {lezerLanguage: lezer} = await load('@codemirror/lang-lezer');
    const {wastLanguage: wast} = await load('@codemirror/lang-wast');
    const emit = row => console.log(JSON.stringify(row));

    for (const [stock, other, first, second, leaf] of [
        [json, lezer, '{ "😀": [', '"字"] }', 'Array'],
        [lezer, json, '@top Main { // 😀', ' "字" }', 'Body'],
    ]) {
        const name = stock.name;
        const source = `${first}\r\n${second}`;
        const marker = new NodeProp();
        const root = marker.add(type => type.isTop ? true : undefined);
        const changed = stock.configure({props: [root]});
        const renamed = changed.configure({strict: true}, 'renamed');
        const otherCopy = stock.configure({props: [root]});
        const leafOnly = stock.configure({props: [marker.add(type => type.name === leaf ? true : undefined)]});
        const withFacet = facet => stock.configure({props: [languageDataProp.add(type => type.isTop ? facet : undefined)]});
        const replace = withFacet(other.data);
        const absent = withFacet(null);
        const wastFacet = withFacet(wast.data);
        const style = (name, scope) => HighlightStyle.define([{tag: tags.string, class: name}], {scope});
        const styles = [style('_'), style('S', stock.parser.topNode), style('R', changed.parser.topNode),
            style('D', otherCopy.parser.topNode), style('L', stock), style('C', other), style('W', wast),
            style('B', stock.parser.nodeSet.types.find(type => type.name === leaf)), style('A', absent.parser.topNode)];
        const sample = (step, source, tree, projection, range = [0, tree.length]) => {
            const spans = [];
            const bytes = projection ? projectedOffsets(source).byteAt : at => Buffer.byteLength(source.slice(0, at));
            highlightTree(tree, styles, (from, to, classes) => {
                const [base, ...hits] = classes.split(' ');
                assert.equal(base, '_');
                spans.push([bytes(from), bytes(to), hits]);
            }, ...range);
            assert(spans.length > 0);
            emit({name, step, source, range: range.map(bytes), spans});
        };
        for (const [step, parser] of Object.entries({stock, changed, renamed, leafOnly, replace, absent, wastFacet})) {
            sample(`raw-${step}`, source, parser.parser.parse(source), false);
        }
        const slot = new Compartment();
        const original = EditorState.create({doc: source, extensions: slot.of(stock)});
        const current = original.update({effects: slot.reconfigure(renamed)}).state;
        const at = current.doc.toString().indexOf('字');
        const edit = current.update({changes: {from: at, to: at + 1, insert: 'new'}});
        sample('editor-new', source, syntaxTree(current), true);
        sample('editor-retained', source, syntaxTree(original), true);
        sample('editor-edit', projectInput(source, edit), syntaxTree(edit.state), true);
        sample('editor-independent', source, syntaxTree(current.update({effects: slot.reconfigure(otherCopy)}).state), true);

        const mounted = `Prose 😀\r\n\r\n> \`\`\`key\r\n> ${first}\r\n> ${second}\r\n> \`\`\``;
        const mixed = EditorState.create({doc: mounted, extensions: markdown({codeLanguages: info => info === 'key' ? changed : null})});
        sample('mount', mounted, syntaxTree(mixed), true);
        const clip = mixed.doc.toString().indexOf('字');
        sample('mount-clipped', mounted, syntaxTree(mixed), true, [clip, clip + 1]);
    }
} finally {
    rmSync(scratch, {recursive: true, force: true});
}
