// node crates/codemirror/tests/fixtures/language_data_modern.mjs <archive-root>
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {projectedOffsets, projectInput} from './editor_line_offsets.mjs';
import {pinnedModules} from './pinned_modules.mjs';

assert.equal(process.argv.length, 3);
const root = mkdtempSync(join(realpathSync(tmpdir()), 'language-data-modern-'));
try {
    const {load} = pinnedModules(resolve(process.argv[2]), root);
    const {languages} = await load('@codemirror/language-data');
    const {StreamLanguage, foldable, getIndentation, language, syntaxTree} = await load('@codemirror/language');
    const {Compartment, EditorState, Prec} = await load('@codemirror/state');
    const {CompletionContext} = await load('@codemirror/autocomplete');
    const {insertNewlineAndIndent} = await load('@codemirror/commands');
    const {go, goLanguage, localCompletionSource} = await load('@codemirror/lang-go');
    const {jsonLanguage} = await load('@codemirror/lang-json');
    const {wastLanguage} = await load('@codemirror/lang-wast');
    const bundles = new Map(await Promise.all(languages.map(async entry => [entry.name, await entry.load()])));
    const modernEntries = [...bundles].filter(([, bundle]) => !(bundle.language instanceof StreamLanguage))
        .map(([name]) => name);
    const selected = {Go: goLanguage, JSON: jsonLanguage, WebAssembly: wastLanguage};
    for (const [name, stock] of Object.entries(selected)) {
        const bundle = bundles.get(name);
        assert.equal(bundle.language, stock);
        assert.deepEqual(bundle.extension, [stock, bundle.support]);
        assert.equal(stock.extension.length, 2);
        assert.equal(stock.extension[0].facet, language);
        assert.equal(stock.extension[1].facet, EditorState.languageData);
        assert.equal(bundle.support.length, name === 'Go' ? 2 : 0);
    }
    const [fixed, local] = bundles.get('Go').support;
    assert.equal(fixed.facet, goLanguage.data);
    assert.equal(local.facet, goLanguage.data);
    assert.deepEqual(Object.keys(fixed.value), ['autocomplete']);
    assert.notEqual(fixed.value.autocomplete, localCompletionSource);
    assert.deepEqual(local.value, {autocomplete: localCompletionSource});

    const outside = () => null;
    const extension = name => {
        if (name === 'outside') return EditorState.languageData.of(() => [{autocomplete: outside}]);
        if (name === 'fresh Go') return go().extension;
        if (name === 'high JSON') return Prec.high(bundles.get('JSON').extension);
        if (name.startsWith('bare ')) return selected[name.slice(5)].extension;
        return bundles.get(name).extension;
    };
    const triggers = ['  }', '  ]', 'case', 'default', 'case x', ')'];
    function run(name, source, line, completion, steps) {
        const slot = new Compartment();
        let state = EditorState.create({doc: source, extensions: slot.of([])});
        return {name, line, triggers, records: steps.map(([step, spec]) => {
            const before = syntaxTree(state);
            const {extensions, edit, select} = spec;
            const {byteAt: beforeByte} = projectedOffsets(source);
            const from = edit && state.doc.toString().indexOf(edit[0]);
            assert(!edit || from >= 0);
            const to = edit && from + edit[0].length;
            const changes = edit && {from, to, insert: edit[1]};
            const tr = state.update({changes, selection: select === undefined ? undefined : {anchor: select},
                effects: extensions && slot.reconfigure(extensions.map(extension))});
            source = projectInput(source, tr);
            state = tr.state;
            const {byteAt} = projectedOffsets(source);
            const at = completion ? state.doc.toString().indexOf(completion) + completion.length : state.doc.line(line + 1).from;
            const root = syntaxTree(state);
            const start = state.doc.line(line).from, end = state.doc.line(line).to;
            const fold = foldable(state, start, end);
            const data = key => state.languageDataAt(key, at);
            const sources = data('autocomplete').map(callback => {
                if (callback === outside) return null;
                const result = callback(new CompletionContext(state, at, true));
                assert(result, step);
                const labels = result.options.map(option => option.label);
                return {from: byteAt(result.from), func: labels.includes('func'), arg: labels.includes('arg'),
                    local: labels.includes('local'), inner: labels.includes('inner')};
            });
            return {step, extensions, edit: edit && [beforeByte(from), beforeByte(to), edit[1]],
                select: select === undefined ? undefined : beforeByte(select), source, at: byteAt(at),
                head: byteAt(state.selection.main.head), root: root.topNode.name,
                active: Object.keys(selected).filter(key => selected[key].isActiveAt(state, at)),
                reused: root === before, sources,
                comments: data('commentTokens').map(({line, block}) => [line ?? null, block ? [block.open, block.close] : null]),
                brackets: data('closeBrackets').map(({brackets, before, stringPrefixes}) => [brackets ?? null, before ?? null, stringPrefixes ?? []]),
                indentOnInput: data('indentOnInput').map(value => triggers.map(text => value.test(text))),
                indent: getIndentation(state, state.doc.line(line + 1).from) ?? null,
                indentAt: byteAt(state.doc.line(line + 1).from), lineRange: [byteAt(start), byteAt(end)],
                fold: fold ? [byteAt(fold.from), byteAt(fold.to)] : null};
        })};
    }
    const scenarios = [
        run('Go', 'package p\r\nfunc f(arg int) {\r\n  var local = 1\r\n  // 字😀\r\n  return local\r\n}\r\n', 2, 'return ', [
            ['bare parser', {extensions: ['bare Go']}],
            ['install catalog bundle', {extensions: ['Go']}],
            ['select without parsing again', {select: 12}],
            ['same extension twice', {extensions: ['Go', 'Go']}],
            ['two independent Go bundles', {extensions: ['Go', 'outside', 'fresh Go']}],
            ['inactive parser also looks up active Go data', {extensions: ['Go', 'outside', 'JSON']}],
            ['high precedence JSON disables Go sources', {extensions: ['Go', 'outside', 'high JSON']}],
            ['switch to Go and edit the local name', {extensions: ['Go'], edit: ['local', 'inner']}],
            ['remove sources but retain parser', {extensions: ['bare Go']}],
            ['detach', {extensions: []}],
        ]),
        run('JSON', '{\r\n  "key": "字😀",\r\n  "ok": true\r\n}', 1, null, [
            ['stock parser', {extensions: ['bare JSON']}],
            ['install catalog bundle twice', {extensions: ['JSON', 'JSON']}],
            ['edit Unicode value', {edit: ['"字😀"', '42']}],
            ['detach', {extensions: []}],
            ['reattach', {extensions: ['JSON']}],
        ]),
        run('WebAssembly', '(module\r\n  ;; 字😀\r\n  (func $run)\r\n)', 1, null, [
            ['stock parser', {extensions: ['bare WebAssembly']}],
            ['install catalog bundle twice', {extensions: ['WebAssembly', 'WebAssembly']}],
            ['edit Unicode comment', {edit: ['字😀', 'updated']}],
            ['detach', {extensions: []}],
            ['reattach', {extensions: ['WebAssembly']}],
        ]),
    ];
    const editor = [
        ['Go', 'package p\nfunc f() {|}'],
        ['JSON', '{|}'],
        ['WebAssembly', '(module|)'],
    ].map(([name, marked]) => {
        const at = marked.indexOf('|'), source = marked.replace('|', '');
        const view = {
            state: EditorState.create({doc: source, selection: {anchor: at}, extensions: extension(name)}),
            dispatch(tr) { view.state = tr.state; },
        };
        assert(insertNewlineAndIndent(view));
        const after = view.state.doc.toString();
        return {name, source, at: projectedOffsets(source).byteAt(at), after,
            head: projectedOffsets(after).byteAt(view.state.selection.main.head)};
    });
    console.log(`{\n  "modernEntries":${JSON.stringify(modernEntries)},\n  "scenarios":[${scenarios.map(({records, ...rest}) =>
        `\n    {${JSON.stringify(rest).slice(1, -1)},"records":[\n      ${records.map(value => JSON.stringify(value)).join(',\n      ')}\n    ]}`).join(',')}\n  ],\n  "editor":[\n    ${editor.map(value => JSON.stringify(value)).join(',\n    ')}\n  ]\n}`);
} finally {
    rmSync(root, {recursive: true, force: true});
}
