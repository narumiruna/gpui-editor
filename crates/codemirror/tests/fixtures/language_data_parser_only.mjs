// node crates/codemirror/tests/fixtures/language_data_parser_only.mjs <archive-root>
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {projectedOffsets, projectInput} from './editor_line_offsets.mjs';
import {pinnedModules} from './pinned_modules.mjs';

assert.equal(process.argv.length, 3);
const root = mkdtempSync(join(realpathSync(tmpdir()), 'language-data-parser-only-'));
try {
    const {load} = pinnedModules(resolve(process.argv[2]), root);
    const {languages} = await load('@codemirror/language-data');
    const {foldable, getIndentation, language, syntaxTree} = await load('@codemirror/language');
    const {Compartment, EditorState, Prec} = await load('@codemirror/state');
    const {cpp, cppLanguage} = await load('@codemirror/lang-cpp');
    const {java, javaLanguage} = await load('@codemirror/lang-java');
    const {rust, rustLanguage} = await load('@codemirror/lang-rust');
    const {yaml, yamlLanguage} = await load('@codemirror/lang-yaml');
    const entries = [
        ['C', cppLanguage, cpp, true],
        ['C++', cppLanguage, cpp, true],
        ['Java', javaLanguage, java, false],
        ['Rust', rustLanguage, rust, true],
        ['YAML', yamlLanguage, yaml, false],
    ];
    const bundles = new Map();
    for (const [name, stock, factory, prefixes] of entries) {
        const entry = languages.find(description => description.name === name);
        const attempt = entry.load();
        assert.equal(entry.load(), attempt);
        const bundle = await attempt;
        assert.equal(await entry.load(), bundle);
        assert.equal(bundle.language, stock);
        assert.deepEqual(bundle.extension, [stock, bundle.support]);
        assert.deepEqual(bundle.support, []);
        assert.equal(stock.extension.length, 2);
        assert.equal(stock.extension[0].facet, language);
        assert.equal(stock.extension[1].facet, EditorState.languageData);
        const state = EditorState.create({extensions: [bundle, factory(), stock]});
        assert.equal(state.facet(language), stock);
        assert.deepEqual(state.facet(EditorState.languageData), [stock.extension[1].value]);
        const keys = ['commentTokens', 'indentOnInput', ...(prefixes ? ['closeBrackets'] : [])];
        assert.deepEqual(Object.keys(state.facet(stock.data)[0]).sort(), keys.sort());
        bundles.set(name, bundle);
    }
    assert.notEqual(bundles.get('C'), bundles.get('C++'));
    assert.equal(bundles.get('C').language.parser, bundles.get('C++').language.parser);
    assert.equal(bundles.get('C').language.extension[1], bundles.get('C++').language.extension[1]);
    assert.equal(new Set([...bundles.values()].map(bundle => bundle.language.parser)).size, 4);
    assert.equal(new Set([...bundles.values()].map(bundle => bundle.language.extension[1])).size, 4);

    const outside = EditorState.languageData.of(() => [{commentTokens: {line: 'outside'}}]);
    function extension(name) {
        if (name === 'outside') return outside;
        if (name.startsWith('high ')) return Prec.high(extension(name.slice(5)));
        return bundles.get(name).extension;
    }

    const triggers = ['  }', '\t{', '  ]', 'case ', 'case', 'case x', ' default:', 'default', ' } ', 'x}', '\ufeff}', '\u0085}'];
    function run(name, source, steps, lines = []) {
        const slot = new Compartment();
        let state = EditorState.create({doc: source, extensions: slot.of([])});
        const records = steps.map(([step, {extensions, edit, select, userEvent}]) => {
            const before = syntaxTree(state);
            const {byteAt: beforeByte} = projectedOffsets(source);
            const match = edit && state.doc.toString().indexOf(edit[0]);
            assert(!edit || match >= 0, `${name}: ${step}`);
            const from = edit && match + (edit[2] ?? 0);
            const to = edit && match + edit[0].length;
            const tr = state.update({
                changes: edit && {from, to, insert: edit[1]},
                selection: select === undefined ? undefined : {anchor: select},
                effects: extensions && slot.reconfigure(extensions.map(extension)),
                userEvent,
            });
            source = projectInput(source, tr);
            state = tr.state;
            const {byteAt} = projectedOffsets(source);
            const tree = syntaxTree(state);
            const active = entries.filter(([, stock]) => stock.isActiveAt(state, 0)).map(([entry]) => entry);
            function languageData(at) {
                const data = key => state.languageDataAt(key, at);
                return {
                    comments: data('commentTokens').map(({line, block}) => [line ?? null, block ? [block.open, block.close] : null]),
                    brackets: data('closeBrackets').map(({brackets, before, stringPrefixes}) => [brackets ?? null, before ?? null, stringPrefixes ?? []]),
                    indentOnInput: data('indentOnInput').map(rule => triggers.map(text => rule.test(text))),
                    autocomplete: data('autocomplete').length,
                    wordChars: data('wordChars'),
                };
            }
            const positions = [...new Set([0, state.doc.line(2).from, state.doc.length])];
            const data = languageData(0);
            for (const at of positions) assert.deepEqual(languageData(at), data, `${name}: ${step}`);
            assert.equal(data.autocomplete, 0);
            assert.deepEqual(data.wordChars, []);
            const services = lines.map(line => {
                const {from, to} = state.doc.line(line);
                const fold = foldable(state, from, to);
                return {lineRange: [byteAt(from), byteAt(to)], indent: getIndentation(state, from) ?? null,
                    fold: fold ? [byteAt(fold.from), byteAt(fold.to)] : null};
            });
            return {step, extensions, edit: edit && [beforeByte(from), beforeByte(to), edit[1]],
                select: select === undefined ? undefined : byteAt(select), userEvent, source,
                head: byteAt(state.selection.main.head), root: tree.topNode.name, active, reused: tree === before,
                positions: positions.map(byteAt), providers: state.facet(EditorState.languageData).length, data, services};
        });
        return {name, records};
    }

    const lifecycle = name => [
        ['install', {extensions: [name]}],
        ['selection only', {select: 2}],
        ['duplicate', {extensions: [name, name]}],
        ['edit Unicode', {edit: ['字😀', 'updated value']}],
        ['detach and edit', {extensions: [], edit: ['updated value', 'new 😀']}],
        ['reattach', {extensions: [name]}],
    ];
    const scenarios = [
        run('C and C++ identity and precedence', '/* 字😀\r\n   note\r*/', [
            ['install C', {extensions: ['C']}],
            ['switch to C++ without parsing', {extensions: ['C++']}],
            ['aliases deduplicate around outside data', {extensions: ['C', 'outside', 'C++']}],
            ['first C parser wins', {extensions: ['C', 'outside', 'Java']}],
            ['first Java parser wins', {extensions: ['Java', 'outside', 'C']}],
            ['high alias moves the shared provider', {extensions: ['Java', 'outside', 'C', 'high C++']}],
            ['higher YAML wins during an edit', {extensions: ['C', 'outside', 'Rust', 'high YAML'], edit: ['字😀', 'other 😺']}],
            ['remove the winner', {extensions: ['Rust', 'outside', 'C']}],
            ['detach parsers but keep outside data', {extensions: ['outside']}],
            ['detach all', {extensions: []}],
        ]),
        run('C++', 'namespace sample {\r\n  const char *s = "字😀";\r}\n', lifecycle('C++'), [1, 2, 3]),
        run('Java', 'class Sample {\r\n  String s = "字😀";\r}\n', lifecycle('Java'), [1, 2, 3]),
        run('Rust', 'fn sample() {\r\n  let s = r"字😀";\r}\n', lifecycle('Rust'), [1, 2, 3]),
        run('YAML', 'root: [\r\n  "字😀",\r  1\r\n]\n', lifecycle('YAML'), [1, 2, 4]),
        run('metadata does not enable input effects', 'fn sample() {\r\n      \n// 字😀', [
            ['install Rust with caret after extra spaces', {extensions: ['Rust'], select: 20}],
            ['typing a closer does not reindent', {edit: ['      ', '}', 6], select: 21, userEvent: 'input.type'}],
        ]),
    ];
    assert(scenarios.at(-1).records.at(-1).source.includes('\r\n      }\n'));
    const triggerJson = JSON.stringify(triggers).replace(/\u0085|\ufeff/g, ch => `\\u${ch.charCodeAt(0).toString(16).padStart(4, '0')}`);
    console.log(`{\n  "triggers":${triggerJson},\n  "scenarios":[${scenarios.map(({name, records}) =>
        `\n    {"name":${JSON.stringify(name)},"records":[\n      ${records.map(record => JSON.stringify(record)).join(',\n      ')}\n    ]}`).join(',')}\n  ]\n}`);
} finally {
    rmSync(root, {recursive: true, force: true});
}
