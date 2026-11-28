// node crates/codemirror/tests/fixtures/language_data_xml.mjs <archive-root>
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {projectedOffsets, projectInput} from './editor_line_offsets.mjs';
import {pinnedModules} from './pinned_modules.mjs';

assert.equal(process.argv.length, 3);
const root = mkdtempSync(join(realpathSync(tmpdir()), 'language-data-xml-'));
try {
    const {load} = pinnedModules(resolve(process.argv[2]), root);
    const {languages} = await load('@codemirror/language-data');
    const xml = await load('@codemirror/lang-xml');
    await load('@codemirror/lang-json');
    const {CompletionContext} = await load('@codemirror/autocomplete');
    const {Compartment, EditorState, Transaction} = await load('@codemirror/state');
    const {EditorView} = await load('@codemirror/view');
    const {foldable, getIndentation, languageDataProp, syntaxTree} = await load('@codemirror/language');

    assert.deepEqual(Object.keys(xml), ['autoCloseTags', 'completeFromSchema', 'xml', 'xmlLanguage']);
    const entry = languages.find(entry => entry.name === 'XML');
    const bundle = await entry.load();
    assert.equal(await entry.load(), bundle);
    assert.equal(bundle.language, xml.xmlLanguage);
    assert.equal(bundle.language.parser.topNode.name, 'Document');
    assert.equal(bundle.language.parser.topNode.prop(languageDataProp), bundle.language.data);
    assert.deepEqual(bundle.extension, [bundle.language, bundle.support]);
    assert.equal(bundle.support.length, 2);
    const [registration, handler] = bundle.support;
    assert.equal(registration.facet, bundle.language.data);
    assert.deepEqual(Object.keys(registration.value), ['autocomplete']);
    assert.equal(handler, xml.autoCloseTags);
    assert.equal(handler.facet, EditorView.inputHandler);

    const json = await languages.find(entry => entry.name === 'JSON').load();
    const outside = EditorState.languageData.of(() => [{autocomplete: () => null}]);
    const configured = xml.xml({elements: [{name: 'custom', top: true}]});
    const disabled = xml.xml({elements: [{name: 'other', top: true}], autoCloseTags: false});
    assert.equal(configured.language, bundle.language);
    assert.equal(disabled.language, bundle.language);
    assert.equal(configured.support[1], handler);
    assert.equal(disabled.support.length, 1);
    assert.notEqual(configured.support[0].value.autocomplete, disabled.support[0].value.autocomplete);
    const schemaSlot = new Compartment();
    const oldState = EditorState.create({doc: '<', extensions: schemaSlot.of(configured)});
    const combined = oldState.update({effects: schemaSlot.reconfigure([disabled, configured, configured])}).state;
    assert.equal(syntaxTree(oldState), syntaxTree(combined));
    assert.deepEqual(combined.languageDataAt('autocomplete', 1, -1),
        [disabled, configured].map(bundle => bundle.support[0].value.autocomplete));
    assert.deepEqual(combined.facet(EditorView.inputHandler), [handler.value]);
    const extensions = {XML: bundle.extension, JSON: json.extension, outside,
        configured: configured.extension, disabled: disabled.extension};
    const print = row => console.log(JSON.stringify(row));
    const normalize = source => source.replace(/\r\n?/g, '\n');
    function unmark(marked) {
        const [before, after] = marked.split('|');
        return [before + after, normalize(before).length];
    }
    function complete(state, source, at, explicit) {
        const {byteAt} = projectedOffsets(source);
        return state.languageDataAt('autocomplete', at, -1).map(source => {
            const result = source(new CompletionContext(state, at, explicit));
            if (source === registration.value.autocomplete) {
                assert.deepEqual(result, xml.completeFromSchema([], [])(new CompletionContext(state, at, explicit)));
            }
            if (!result) return null;
            assert(result.validFor instanceof RegExp);
            return {from: byteAt(result.from), to: result.to == null ? null : byteAt(result.to),
                filter: result.filter ?? true, options: result.options.map(({label, type, boost, ...rest}) => {
                    assert.deepEqual(rest, {});
                    return {label, type, boost: boost ?? 0};
                })};
        });
    }
    for (const [marked, explicit] of [
        ['<a>α &am|', false], ['<a>α &am|', true], ['<|', false], ['<a at|', false],
        ['<a x="on|e">', false], ['<a x="on|e">', true], ['😀\r\n<名>α</名|>', false],
        ['<a>|</a>', true],
    ]) {
        const [source, at] = unmark(marked);
        const state = EditorState.create({doc: source, extensions: bundle});
        assert.deepEqual(state.languageDataAt('autocomplete', at, -1), [registration.value.autocomplete]);
        print({kind: 'query', marked, source, at: projectedOffsets(source).byteAt(at), explicit,
            results: complete(state, source, at, explicit)});
    }

    const slot = new Compartment();
    let source = '😀\r\n<名 x="one">\r\n  <child/>\r\n</名>';
    let state = EditorState.create({doc: source, extensions: slot.of([])});
    const triggers = ['  </', '\t</', '</ ', '<', '  >'];
    for (const [step, names, select, edit] of [
        ['install', ['XML']], ['selection only', null, 2], ['repeat bundle', ['XML', 'XML']],
        ['outside after', ['XML', 'outside']], ['outside before', ['outside', 'XML']],
        ['edit', null, undefined, ['one', 'α🧪']], ['XML inactive', ['JSON', 'XML', 'outside']],
        ['detach', ['outside']],
    ]) {
        const original = projectedOffsets(source), previous = syntaxTree(state);
        const from = edit && state.sliceDoc().indexOf(edit[0]), to = edit && from + edit[0].length;
        assert(!edit || from >= 0);
        const tr = state.update({changes: edit && {from, to, insert: edit[1]},
            selection: select === undefined ? undefined : {anchor: select},
            effects: names ? slot.reconfigure(names.map(name => extensions[name])) : undefined});
        source = projectInput(source, tr);
        state = tr.state;
        const open = state.doc.line(2), at = open.to, indentAt = state.doc.line(3).from;
        const {byteAt} = projectedOffsets(source);
        const tree = syntaxTree(state), active = xml.xmlLanguage.isActiveAt(state, at, -1);
        const nodes = [];
        if (active) tree.iterate({enter(node) { nodes.push([node.name, byteAt(node.from), byteAt(node.to)]); }});
        const data = key => state.languageDataAt(key, at, -1);
        const fold = foldable(state, open.from, open.to);
        const handlers = names?.includes('XML') ?? true;
        assert.deepEqual(state.facet(EditorView.inputHandler), handlers ? [handler.value] : []);
        print({kind: 'state', step, names, source, at: byteAt(at), head: byteAt(state.selection.main.head),
            select: select === undefined ? undefined : original.byteAt(select),
            edit: edit && [original.byteAt(from), original.byteAt(to), edit[1]],
            active, nodes, reused: active && previous === tree,
            comments: data('commentTokens').map(({line, block}) => [line ?? null, block && [block.open, block.close]]),
            triggers: data('indentOnInput').map(rule => triggers.map(text => rule.test(text))),
            results: complete(state, source, at, true), lineRange: [byteAt(open.from), byteAt(open.to)],
            indentAt: byteAt(indentAt), indent: active ? getIndentation(state, indentAt) : null,
            fold: active && fold ? [byteAt(fold.from), byteAt(fold.to)] : null});
    }

    for (const [mode, marked, typed, names = ['XML']] of [
        ['open', '<a|', '>', ['XML', 'XML']], ['rewritten default', '😀\r\n<Old|', '>'],
        ['caret before input', '<a|', '>'], ['slash replaces suffix', '😀\r\n<名><|>tail', '/'],
        ['existing close', '<a|</a>', '>'], ['filtered completion', '<a|', '>'],
        ['range replacement', '<aa|', '>'], ['unrelated input', '<a|', 'x'],
        ['inactive', '<a|', '>', ['JSON', 'XML']], ['detached', '<a|', '>', []],
        ['configured open', '<custom|', '>', ['configured', 'disabled', 'XML', 'configured']],
        ['disabled open', '<other|', '>', ['disabled']],
        ['disabled slash', '<other><|>tail', '/', ['disabled']],
        ['read only', '<a|', '>'], ['composition', '<a|', '>'],
    ]) {
        const [source, at] = unmark(marked);
        const {byteAt} = projectedOffsets(source);
        const from = mode === 'range replacement' ? at - 1 : at;
        const cursor = mode === 'caret before input' ? at : from + typed.length;
        const rewrite = mode === 'rewritten default' ? {from: at - 3, to: at, insert: '名>'} : null;
        const typedSpec = (changes, anchor) => ({changes, selection: {anchor},
            userEvent: 'input.type', scrollIntoView: true});
        const state = EditorState.create({doc: source, selection: {anchor: at}, extensions: [
            names.map(name => extensions[name]), EditorState.readOnly.of(mode === 'read only'),
            EditorState.changeFilter.of(tr => mode !== 'filtered completion' || !tr.isUserEvent('input.complete')),
            EditorState.transactionFilter.of(tr => rewrite && tr.isUserEvent('input.type')
                ? typedSpec(rewrite, rewrite.from + rewrite.insert.length) : tr),
        ]});
        let transactions = [], calls = 0, cached;
        const view = {state, composing: mode === 'composition', dispatch(batch) { transactions = batch; }};
        const base = () => cached ??= state.update(typedSpec({from, to: at, insert: typed}, cursor));
        const handled = state.facet(EditorView.inputHandler).some(handler => handler(view, from, at, typed, () => {
            calls++;
            return base();
        }));
        assert.equal(handled, ['open', 'rewritten default', 'slash replaces suffix', 'filtered completion',
            'configured open'].includes(mode));
        assert.equal(calls, Number(handled || ['caret before input', 'existing close'].includes(mode)));
        if (mode === 'composition' || mode === 'read only') continue;
        if (!handled) transactions = [base()];
        let before = source;
        const steps = transactions.map(tr => {
            const {byteAt} = projectedOffsets(before), changes = [];
            tr.changes.iterChanges((from, to, _fromB, _toB, insert) => {
                changes.push([byteAt(from), byteAt(to), insert.toString()]);
            });
            const after = projectInput(before, tr), {head, assoc} = tr.state.selection.main;
            const result = {before, after, head: projectedOffsets(after).byteAt(head), assoc,
                event: tr.annotation(Transaction.userEvent), scroll: tr.scrollIntoView, changes};
            before = after;
            return result;
        });
        print({kind: 'input', mode, names, source, at: byteAt(at), from: byteAt(from), typed,
            cursor: byteAt(from) + (cursor - from), rewrite: rewrite && [byteAt(rewrite.from), byteAt(rewrite.to), rewrite.insert],
            steps});
    }
} finally {
    rmSync(root, {recursive: true, force: true});
}
