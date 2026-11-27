// Run with Node and a directory of local SHA-512-pinned npm archives.
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {projectedOffsets, projectInput} from './editor_line_offsets.mjs';
import {pinnedModules} from './pinned_modules.mjs';

assert.equal(process.argv.length, 3, 'provide the directory of local npm archives');
const scratch = mkdtempSync(join(realpathSync(tmpdir()), 'lr-style-tags.'));
try {
    const {load} = pinnedModules(resolve(process.argv[2]), scratch);
    const {EditorState} = await load('@codemirror/state');
    const {languageDataProp, syntaxTree} = await load('@codemirror/language');
    const {styleTags, getStyleTags, highlightTree, tags} = await load('@lezer/highlight');
    const {markdown} = await load('@codemirror/lang-markdown');
    const {jsonLanguage: json} = await load('@codemirror/lang-json');
    const {lezerLanguage: lezer} = await load('@codemirror/lang-lezer');
    const names = new Map(Object.entries(tags).map(([name, tag]) => [tag, name]));
    names.set(tags.definition(tags.variableName), 'definition(variableName)');
    const tagName = tag => {
        const name = names.get(tag);
        assert(name, `unrecorded tag ${tag}`);
        return name;
    };
    const props = specs => styleTags(Object.fromEntries(specs.map(([selector, names]) =>
        [selector, names.map(name => tags[name])])));
    const rules = (tree, byteAt) => {
        const result = [];
        tree.iterate({enter(node) {
            const rule = getStyleTags(node);
            if (rule) result.push([node.name, byteAt(node.from), byteAt(node.to), rule.tags.map(tagName), rule.opaque, rule.inherit]);
        }});
        return result;
    };
    const clips = (tree, source, clipAt, scope) => {
        const {byteAt, units} = projectedOffsets(source);
        const spans = ([from, to]) => {
            const spans = [];
            highlightTree(tree, {scope, style: list => list.map(tagName).join(' ') || null},
                (start, end, classes) => spans.push([byteAt(start), byteAt(end), classes.split(' ')]), from, to);
            return [byteAt(from), byteAt(to), spans];
        };
        return [[0, units], [clipAt, clipAt + 1], [clipAt, clipAt]].map(spans);
    };

    for (const [stock, source, a, b, suppress] of [
        [json, '{\r\n  "名😀": [0, {"x":"אב"}, null], "keep": true\r\n}', [
            ['JsonText/...', ['meta']], ['Array/...', ['emphasis']],
            ['Object/Property/PropertyName', ['name', 'operator']], ['PropertyName', ['atom']],
            ['Array/Number', ['labelName']], ['*/Number', ['literal']], ['String', []],
            ['Array/Object!', ['comment']], ['Array/"[" Array/"]"', ['separator']],
        ], [
            ['Array/Number', ['className']], ['Object/PropertyName', ['propertyName']],
        ], ['Array/Object!', []]],
        [lezer, '@top Main { word+ "名😀" }\r\nword { "x" }\r\n@tokens { Word { @asciiLetter+ } }', [
            ['Declaration/...', ['meta']], ['Declaration/Body/...', ['strong']], ['Body/...', ['emphasis']],
            ['RuleDeclaration/RuleName', ['labelName']], ['Expression', ['name']],
            ['Body/Literal', []], ['TokensBody!', ['comment']],
            ['Body/"{" Body/"}"', ['separator']],
        ], [
            ['RuleDeclaration/RuleName', ['className']], ['*/Literal', ['string']],
        ], ['TokensBody!', []]],
    ]) {
        const name = stock.name;
        const observe = (step, sources, lang) => {
            const state = EditorState.create({doc: source, extensions: lang});
            const at = state.doc.toString().indexOf('名');
            const after = state.update({changes: {from: at, to: at + 1, insert: '字字'}});
            const edited = projectInput(source, after);
            const tree = syntaxTree(state);
            const raw = lang.parser.parse(source);
            console.log(JSON.stringify({name, step, source, sources,
                raw: rules(raw, at => Buffer.byteLength(source.slice(0, at))),
                rules: rules(tree, projectedOffsets(source).byteAt), clips: clips(tree, source, at),
                edited, edit: projectedOffsets(source).byteAt(at), editClips: clips(syntaxTree(after.state), edited, at),
            }));
        };
        for (const [step, sources] of [['stock', []], ['inherited', [a]], ['precedence', [a, b]], ['opaque-empty', [a, b, [suppress]]]]) {
            observe(step, sources, stock.configure({props: sources.map(props)}));
        }
        const mount = `Prose 😀\r\n\r\n> \`\`\`selected\r\n> ${source.replaceAll('\r\n', '\r\n> ')}\r\n> \`\`\``;
        const sources = [a, b];
        const lang = stock.configure({props: sources.map(props)});
        const state = EditorState.create({doc: mount, extensions: markdown({codeLanguages: info => info === 'selected' ? lang : null})});
        const at = state.doc.toString().indexOf('名');
        console.log(JSON.stringify({name, step: 'mount', source: mount, sources, clips: clips(syntaxTree(state), mount, at, type => type.prop(languageDataProp) === lang.data)}));
    }
} finally {
    rmSync(scratch, {recursive: true, force: true});
}
