// From the repository root, run with Node 22.14+ and a directory of npm archives
// (missing archives are downloaded and all archives are verified by SHA-512):
// node crates/codemirror/tests/fixtures/yaml_frontmatter_configured.mjs /path/to/archives > crates/codemirror/tests/fixtures/yaml_frontmatter_configured.jsonl
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {projectedOffsets} from './editor_line_offsets.mjs';
import {pinnedModules} from './pinned_modules.mjs';

assert.equal(process.argv.length, 3, 'provide the directory of local npm archives');
const scratch = mkdtempSync(join(realpathSync(tmpdir()), 'yaml-frontmatter-configured.'));
try {
    const {load, pins} = pinnedModules(resolve(process.argv[2]), scratch);
    assert.equal(pins.get('@codemirror/lang-yaml').version, '6.1.3');
    const {yamlFrontmatter} = await load('@codemirror/lang-yaml');
    const {jsonLanguage} = await load('@codemirror/lang-json');
    const {Compartment, EditorState} = await load('@codemirror/state');
    const {LanguageSupport, defineLanguageFacet, languageDataProp, syntaxTree, syntaxTreeAvailable} =
        await load('@codemirror/language');
    const {NodeProp} = await load('@lezer/common');
    const {highlightTree, styleTags, tags} = await load('@lezer/highlight');
    const key = new NodeProp();
    const facet = defineLanguageFacet({probe: 'base'});
    const modes = Object.fromEntries(['strict', 'recover'].map((name, i) => {
        const language = jsonLanguage.configure({strict: i === 0, props: [
            key.add(type => type.name === 'Array' ? name : undefined),
            languageDataProp.add(type => type.isTop ? facet : undefined),
            styleTags({PropertyName: i === 0 ? tags.atom : tags.keyword}),
        ]}, name);
        const support = new LanguageSupport(language, facet.of({probe: name}));
        const wrapped = yamlFrontmatter({content: support});
        assert.equal(wrapped.support, support.support);
        return [name, wrapped];
    }));
    const slot = new Compartment(), data = new Compartment();
    const normalize = source => source.replace(/\r\n?/g, '\n');
    const emit = row => console.log(JSON.stringify(row));
    let source = '---\r\ntitle: 😀\r---\r\n \t\r\n{\r\n  "名😀": [1, 2]\r\n}\r\n';
    let state = EditorState.create({doc: source, extensions: [
        data.of(facet.of({probe: 'override'})), slot.of(modes.strict),
    ]});
    function capture(step) {
        const {byteAt} = projectedOffsets(source);
        assert.equal(state.doc.toString(), normalize(source));
        assert.ok(syntaxTreeAvailable(state, state.doc.length));
        const tree = syntaxTree(state);
        const body = tree.topNode.getChild('JsonText');
        assert.ok(body);
        const nodes = [], highlights = [], metadata = [];
        body.cursor().iterate(node => {
            nodes.push([node.name, byteAt(node.from), byteAt(node.to), node.type.prop(key) ?? null]);
        });
        highlightTree(tree, {style(values) {
            if (values.includes(tags.atom)) return 'atom';
            if (values.includes(tags.keyword)) return 'keyword';
            return null;
        }}, (from, to, tag) => highlights.push([byteAt(from), byteAt(to), tag]), body.from, body.to);
        const sites = new Set([0, body.from, body.to, state.doc.toString().indexOf('名')]);
        for (const at of sites) {
            metadata.push([byteAt(at), ...[-1, 1].map(side => state.languageDataAt('probe', at, side))]);
        }
        const front = tree.topNode.getChild('Frontmatter');
        const yamlProbe = front ? byteAt(state.doc.toString().indexOf('title:') + 1) : null;
        if (front) assert.equal(state.languageDataAt('commentTokens', state.doc.toString().indexOf('title:') + 1)[0].line, '#');
        assert.equal(highlights.length, 1);
        assert.ok(nodes.some(node => node[3] !== null));
        emit({...step, source, body: [byteAt(body.from), byteAt(body.to)], nodes, highlights, metadata, yamlProbe});
    }
    function replay(label, {replace, mode, removeData, selection, rejected} = {}) {
        let edit, changes, nextSource = source;
        if (replace) {
            const [before, insert] = replace;
            const from = source.indexOf(before), to = from + before.length;
            assert.ok(from >= 0, before);
            changes = {from: normalize(source.slice(0, from)).length,
                to: normalize(source.slice(0, to)).length, insert: normalize(insert)};
            edit = [Buffer.byteLength(source.slice(0, from)), Buffer.byteLength(source.slice(0, to)), insert];
            nextSource = source.slice(0, from) + insert + source.slice(to);
        }
        const effects = mode ? slot.reconfigure(modes[mode]) : removeData ? data.reconfigure([]) : [];
        const update = () => state.update({changes, effects, selection: selection ? {anchor: selection} : undefined}).state;
        if (rejected) {
            const previous = syntaxTree(state);
            let stop;
            assert.throws(update, error => {
                assert.ok(error instanceof SyntaxError);
                const match = /^No parse at (\d+)$/.exec(error.message);
                assert.ok(match, error.message);
                stop = projectedOffsets(nextSource).byteAt(Number(match[1]));
                return true;
            });
            assert.equal(state.doc.toString(), normalize(source));
            assert.equal(syntaxTree(state), previous);
            emit({label, edit, mode, rejected: true, attempted: nextSource, stop});
            return;
        }
        state = update();
        source = nextSource;
        capture({label, edit, mode, removeData, selection});
    }

    capture({label: 'install'});
    replay('selection only', {selection: 1});
    replay('front edit shifts unchanged body', {replace: ['😀\r---', '新しい😀\r---']});
    replay('edit in body', {replace: ['1, 2', '1, 200']});
    replay('strict rejects body edit', {replace: ['1, 200', '1,'], rejected: true});
    replay('strict rejects losing the opening marker', {replace: ['---\r\n', '----\r\n'], rejected: true});
    replay('rebuild body after crossing old boundary', {replace: ['---\r\n \t\r\n', '\r\n---\r\n']});
    replay('drop override without parsing', {removeData: true});
    replay('valid body fills whole document', {replace: [source.slice(0, source.indexOf('{')), '']});
    replay('restore front matter', {replace: ['{', '---\r\ntitle: 😀\r---\r\n{']});
    replay('switch to configured recovery body', {mode: 'recover'});
    replay('recover without losing properties', {replace: ['1, 200', '1,']});
    replay('strict rejects invalid current body on reconfigure', {mode: 'strict', rejected: true});
    replay('repair and install strict mode together', {replace: ['1,]', '1, 3]'], mode: 'strict'});
    replay('return to recovery configuration', {mode: 'recover'});
    replay('install strict mode with a valid body edit', {replace: ['1, 3', '1, 4'], mode: 'strict'});
    replay('prepare a second invalid recovery body', {replace: ['1, 4', '1,'], mode: 'recover'});
    replay('strict rejects still invalid final body at new offset', {replace: ['1,]', '100,]'], mode: 'strict', rejected: true});
} finally {
    rmSync(scratch, {recursive: true, force: true});
}
