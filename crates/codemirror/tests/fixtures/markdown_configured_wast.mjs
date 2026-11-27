// Run with Node and a directory of local SHA-512-pinned npm archives.
import assert from 'node:assert/strict';
import {existsSync, mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {projectedOffsets} from './editor_line_offsets.mjs';
import {pinnedModules} from './pinned_modules.mjs';

assert.equal(process.argv.length, 3, 'usage: node markdown_configured_wast.mjs <pinned archive directory>');
const archives = resolve(process.argv[2]);
const root = mkdtempSync(join(realpathSync(tmpdir()), 'markdown-configured-wast-'));
try {
    const modules = pinnedModules(archives, root);
    // The loader can fetch missing files. Require local files before loading.
    for (const [name, pin] of modules.pins) {
        assert(existsSync(join(archives, name.replace('/', '__'), `${name.split('/').at(-1)}-${pin.version}.tgz`)), name);
    }
    const {Compartment, EditorState} = await modules.load('@codemirror/state');
    const {defineLanguageFacet, languageDataProp, syntaxTree} = await modules.load('@codemirror/language');
    const {markdown, commonmarkLanguage} = await modules.load('@codemirror/lang-markdown');
    const {wastLanguage: stock} = await modules.load('@codemirror/lang-wast');
    const {NodeProp} = await modules.load('@lezer/common');
    const {highlightTree, styleTags, tagHighlighter, tags} = await modules.load('@lezer/highlight');

    const source = '前 😀\r\n\r\n> ```wat demo\r\n> (; 😀\r\n> λ ;)\r\n> (module 12)\r\n> ```\r\n\r\n```other\r\n(func 34)\r\n```\r\n\r\n```\r\n(func 56)\r\n```\r\n\r\n    (func 78)';
    const {byteAt} = projectedOffsets(source);
    const marker = new NodeProp();
    const named = stock.configure({props: [marker.add({Module: true}), styleTags({Number: tags.bool})]}, 'named');
    const facet = defineLanguageFacet({probe: 'default'});
    const fallback = stock.configure({props: [languageDataProp.add({Module: facet})]});
    const install = (selected, defaultCodeLanguage) => markdown({
        codeLanguages: name => name === 'wat' ? selected : null,
        defaultCodeLanguage,
    });
    const slot = new Compartment();
    const retained = EditorState.create({doc: source, extensions: [
        slot.of(install(named, fallback)), stock.data.of({probe: 'stock'}), facet.of({probe: 'override'}),
    ]});
    const retainedTree = syntaxTree(retained);
    const control = retained.update({effects: slot.reconfigure(install(stock, stock))}).state;
    assert.notEqual(syntaxTree(control), retainedTree);
    assert.equal(syntaxTree(retained), retainedTree);
    const scope = (tag, cls, test) => tagHighlighter([{tag, class: cls}], {scope: test});
    const watched = [tags.bool, tags.number, tags.blockComment];
    const styles = [tagHighlighter(watched.map((tag, i) => ({tag, class: ['bool', 'number', 'blockComment'][i]}))),
        ...watched.flatMap(tag => [
            scope(tag, 'named-row', type => type === named.parser.topNode),
            scope(tag, 'stock-row', type => type === stock.parser.topNode),
            scope(tag, 'stock-facet', type => type.prop(languageDataProp) === stock.data),
            scope(tag, 'fallback-facet', type => type.prop(languageDataProp) === facet),
        ])];
    for (const [step, state] of [['configured', retained], ['stock control', control]]) {
        const text = state.doc.toString();
        const gap = text.indexOf('> λ');
        const positions = [gap, gap + 1, gap + 2, ...['12', '34', '56', '78'].map(word => text.indexOf(word) + 1)];
        const probes = positions.map(pos => [byteAt(pos), ...[-1, 1].map(side => [
            stock.isActiveAt(state, pos, side), commonmarkLanguage.isActiveAt(state, pos, side),
            state.languageDataAt('probe', pos, side),
        ])]);
        const spans = [];
        highlightTree(syntaxTree(state), styles, (from, to, classes) => {
            spans.push([byteAt(from), byteAt(to), classes.split(' ')]);
        });
        const regions = stock.findRegions(state).map(({from, to}) => [byteAt(from), byteAt(to)]);
        console.log(JSON.stringify({step, source, regions, probes, spans}));
    }

    const strict = stock.configure({strict: true});
    for (const [step, source, info] of [
        ['strict named', '前 😀\r\n\r\n> ```wat demo\r\n> ;; 😀\r\n> )\r\n> ```', 'wat'],
        ['strict fallback', '前 😀\r\n\r\n```other\r\n)\r\n```', 'other'],
        ['strict eof', '前 😀\r\n\r\n```wat\r\n(module\r\n```', 'wat'],
    ]) {
        const parser = info === 'wat' ? install(strict, stock) : install(stock, strict);
        assert.throws(() => EditorState.create({doc: source, extensions: parser}), error => {
            const offset = Number(/No parse at (\d+)/.exec(error.message)[1]);
            console.log(JSON.stringify({step, source, errorAt: projectedOffsets(source).byteAt(offset)}));
            return true;
        });
    }
} finally {
    rmSync(root, {recursive: true, force: true});
}
