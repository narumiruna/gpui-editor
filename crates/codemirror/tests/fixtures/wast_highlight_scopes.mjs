// From the repository root: node crates/codemirror/tests/fixtures/wast_highlight_scopes.mjs <archive-root>
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {pinnedModules} from './pinned_modules.mjs';

const root = realpathSync(mkdtempSync(join(tmpdir(), 'wast-highlight-scopes-')));
try {
    const modules = pinnedModules(resolve(process.argv[2]), root);
    const {EditorState, Compartment} = await modules.load('@codemirror/state');
    const {HighlightStyle, defineLanguageFacet, highlightingFor, languageDataProp, sublanguageProp,
        syntaxHighlighting, syntaxTree} = await modules.load('@codemirror/language');
    const {wastLanguage: stock} = await modules.load('@codemirror/lang-wast');
    const {markdown} = await modules.load('@codemirror/lang-markdown');
    const {NodeProp} = await modules.load('@lezer/common');
    const {highlightTree, tags} = await modules.load('@lezer/highlight');

    const input = ' (module (func))  ';
    const marker = new NodeProp();
    const prop = marker.add({Module: true});
    const configured = stock.configure({props: [prop]});
    const configuredRenamed = configured.configure({}, 'configured-renamed');
    const again = stock.configure({props: [prop]});
    const appOnly = stock.configure({props: [marker.add({App: true})]});
    const renamed = stock.configure({}, 'renamed');
    const strict = stock.configure({strict: true}, 'strict-renamed');
    const replacement = defineLanguageFacet({});
    const withFacet = facet => stock.configure({props: [languageDataProp.add(type => type.isTop ? facet : undefined)]});
    const swapped = withFacet(replacement);
    const absent = withFacet(null);
    const inner = defineLanguageFacet({});
    const sub = stock.configure({props: [sublanguageProp.add({
        Module: [{type: 'replace', test: () => true, facet: inner}],
    })]});
    const app = stock.parser.nodeSet.types.find(type => type.name === 'App');
    assert.notEqual(configured.parser.topNode, again.parser.topNode);
    assert.equal(renamed.parser.topNode, stock.parser.topNode);
    assert.equal(configuredRenamed.parser.topNode, configured.parser.topNode);
    assert.equal(swapped.data, stock.data);
    assert.equal(absent.parser.topNode.prop(languageDataProp), null);

    const keyword = (name, scope, themeType) => HighlightStyle.define(
        [{tag: tags.keyword, class: name}], {scope, themeType});
    // The replacement and inner scopes accept only rows with those exact facets.
    const matchesFacet = (name, facet) => ({
        scope: type => type.prop(languageDataProp) === facet,
        style: tagList => tagList.includes(tags.keyword) ? name : null,
    });
    const styles = [keyword('_'), keyword('L', stock), matchesFacet('F', replacement), matchesFacet('I', inner),
        keyword('S', stock.parser.topNode), keyword('R', configured.parser.topNode),
        keyword('D', again.parser.topNode), keyword('A', app), keyword('X', absent.parser.topNode)];
    const sample = (name, tree, text, from = 0, to = tree.length) => {
        const hits = [];
        const byte = at => Buffer.byteLength(text.slice(0, at));
        highlightTree(tree, styles, (a, b, classes) => {
            const [token, ...scopes] = classes.split(' ');
            assert.equal(token, '_');
            hits.push([byte(a), byte(b), text.slice(a, b), scopes]);
        }, from, to);
        console.log(JSON.stringify({name, text, range: [byte(from), byte(to)], hits}));
    };
    const slot = new Compartment();
    const original = EditorState.create({doc: input, extensions: slot.of(stock)});
    const reconfigure = (state, language) => state.update({effects: slot.reconfigure(language)}).state;
    const next = reconfigure(original, configured);
    const edited = next.update({changes: {from: 14, insert: ' $x'}}).state;
    for (const [name, language] of Object.entries({stock, configured, 'configured-renamed': configuredRenamed,
        'app-only': appOnly, renamed, strict, swapped, absent})) {
        sample(`raw-${name}`, language.parser.parse(input), input);
    }
    for (const [name, state] of Object.entries({original, next, edited, again: reconfigure(edited, again),
        renamed: reconfigure(next, configuredRenamed), swapped: reconfigure(original, swapped),
        absent: reconfigure(original, absent), sub: reconfigure(original, sub), retained: original})) {
        sample(`attached-${name}`, syntaxTree(state), state.doc.toString());
    }

    const md = '😀 prose\n\n> ```webassembly\n> (module\n>  (func))\n> ```\n\n```unknown\n(module)\n```';
    const mixed = EditorState.create({doc: md, extensions: markdown({codeLanguages: info => info === 'webassembly' ? stock : null})});
    sample('markdown', syntaxTree(mixed), md);
    sample('markdown-clipped', syntaxTree(mixed), md, md.indexOf('module') + 3, md.indexOf('func') + 2);

    const fallback = syntaxHighlighting(keyword('fallback', stock, 'dark'), {fallback: true});
    const miss = syntaxHighlighting(keyword('miss', configured.parser.topNode));
    const wrongTheme = syntaxHighlighting(keyword('miss', stock, 'dark'));
    const choices = [[fallback], [miss, fallback], [wrongTheme, fallback]].map(extensions =>
        highlightingFor(EditorState.create({extensions}), [tags.keyword], stock.parser.topNode));
    console.log(JSON.stringify({name: 'fallback', choices}));
} finally {
    rmSync(root, {recursive: true, force: true});
}
