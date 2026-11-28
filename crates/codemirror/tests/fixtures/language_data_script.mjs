// node crates/codemirror/tests/fixtures/language_data_script.mjs <archive-root>
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {projectedOffsets, projectInput} from './editor_line_offsets.mjs';
import {pinnedModules} from './pinned_modules.mjs';

assert.equal(process.argv.length, 3);
const root = mkdtempSync(join(realpathSync(tmpdir()), 'language-data-script-'));
try {
    const {load} = pinnedModules(resolve(process.argv[2]), root);
    const {languages} = await load('@codemirror/language-data');
    const py = await load('@codemirror/lang-python');
    const js = await load('@codemirror/lang-javascript');
    const {CompletionContext} = await load('@codemirror/autocomplete');
    const {Compartment, EditorState, Prec, Transaction} = await load('@codemirror/state');
    const {EditorView} = await load('@codemirror/view');
    const {foldable, getIndentation, language, languageDataProp, syntaxTree} = await load('@codemirror/language');
    const parsers = new Map(Object.entries({Python: py.pythonLanguage, JavaScript: js.javascriptLanguage,
        TypeScript: js.typescriptLanguage, JSX: js.jsxLanguage, TSX: js.tsxLanguage}));
    const bundles = new Map(), fixed = new Set();
    for (const [name, parser] of parsers) {
        const entry = languages.find(entry => entry.name === name);
        const bundle = await entry.load();
        assert.equal(await entry.load(), bundle);
        assert.equal(bundle.language, parser);
        assert.equal(parser.parser.topNode.name, 'Script');
        assert.equal(parser.parser.topNode.prop(languageDataProp), parser.data);
        assert.equal(parser.data === js.javascriptLanguage.data, name !== 'Python');
        assert.deepEqual(bundle.extension, [parser, bundle.support]);
        const [first, second, handler] = bundle.support;
        for (const input of [first, second]) {
            assert.equal(input.facet, parser.data);
            assert.deepEqual(Object.keys(input.value), ['autocomplete']);
        }
        if (name === 'Python') {
            assert.equal(bundle.support.length, 2);
            assert.equal(first.value.autocomplete, py.localCompletionSource);
            assert.equal(second.value.autocomplete, py.globalCompletion);
        } else {
            assert.equal(bundle.support.length, 3);
            assert.equal(second.value.autocomplete, js.localCompletionSource);
            assert.notEqual(first.value.autocomplete, js.localCompletionSource);
            assert(!fixed.has(first.value.autocomplete));
            fixed.add(first.value.autocomplete);
            if (name === 'JSX' || name === 'TSX') {
                assert.equal(handler, js.autoCloseTags);
                assert.equal(handler.facet, EditorView.inputHandler);
            } else {
                assert.deepEqual(handler, []);
            }
        }
        bundles.set(name, bundle);
    }
    assert.equal(bundles.get('JSX').support[2], bundles.get('TSX').support[2]);

    const outside = () => null;
    const extension = name => {
        if (name === 'outside') return EditorState.languageData.of(() => [{autocomplete: outside}]);
        if (name.startsWith('high ')) return Prec.high(extension(name.slice(5)));
        return bundles.get(name).extension;
    };
    const triggers = ['  }', '  ]', 'case ', 'case x:', 'else:', 'elif ', '  default:', '  </'];
    function summarize(result, byteAt) {
        if (!result) return null;
        let hash = 0xcbf29ce484222325n;
        for (const {label, type, detail, boost, apply, ...rest} of result.options) {
            assert.deepEqual(rest, {});
            assert(apply === undefined || typeof apply === 'function');
            const bytes = Buffer.from(`${label}\0${type ?? ''}\0${detail ?? ''}\0${boost ?? 0}\0${Number(!!apply)}\n`);
            for (const byte of bytes) hash = BigInt.asUintN(64, (hash ^ BigInt(byte)) * 0x100000001b3n);
        }
        assert(result.validFor instanceof RegExp);
        return {from: byteAt(result.from), to: result.to == null ? null : byteAt(result.to),
            filter: result.filter ?? true, count: result.options.length, hash: hash.toString(16).padStart(16, '0')};
    }
    function run(name, source, steps) {
        const slot = new Compartment();
        let state = EditorState.create({doc: source, extensions: slot.of([])});
        for (const [step, {extensions, select, edit}] of steps) {
            const previous = syntaxTree(state), {byteAt: beforeByte} = projectedOffsets(source);
            const from = edit && state.doc.toString().indexOf(edit[0]);
            assert(!edit || from >= 0);
            const to = edit && from + edit[0].length;
            const tr = state.update({changes: edit && {from, to, insert: edit[1]},
                selection: select === undefined ? undefined : {anchor: select},
                effects: extensions && slot.reconfigure(extensions.map(extension))});
            source = projectInput(source, tr);
            state = tr.state;
            const {byteAt} = projectedOffsets(source), text = state.doc.toString();
            const at = text.indexOf('return ') + 'return '.length + 2;
            const inComment = state.doc.line(1).to;
            const open = state.doc.line(2), indentAt = state.doc.line(3).from;
            const active = state.facet(language);
            const tree = syntaxTree(state);
            const fold = foldable(state, open.from, open.to);
            const data = key => state.languageDataAt(key, at);
            const queries = [[at, true], [at, false], [inComment, true]].map(([position, explicit]) => ({
                at: byteAt(position), explicit, results: state.languageDataAt('autocomplete', position).map(source =>
                    summarize(source(new CompletionContext(state, position, explicit)), byteAt)),
            }));
            const activeFacet = [...parsers].filter(([, parser]) => parser.isActiveAt(state, at)).map(([name]) => name);
            console.log(JSON.stringify({kind: 'state', name, step, source, extensions, at: byteAt(at),
                edit: edit && [beforeByte(from), beforeByte(to), edit[1]], select: select === undefined ? undefined : beforeByte(select),
                head: byteAt(state.selection.main.head), root: tree.topNode.name,
                active: [...parsers].find(([, parser]) => parser === active)?.[0] ?? null, activeFacet,
                reused: !!active && previous === tree, queries,
                comments: data('commentTokens').map(({line, block}) => [line ?? null, block ? [block.open, block.close] : null]),
                brackets: data('closeBrackets').map(({brackets, before, stringPrefixes}) => [brackets ?? null, before ?? null, stringPrefixes ?? []]),
                wordChars: data('wordChars'), triggers: data('indentOnInput').map(rule => triggers.map(value => rule.test(value))),
                lineRange: [byteAt(open.from), byteAt(open.to)], indentAt: byteAt(indentAt),
                indent: getIndentation(state, indentAt) ?? null, fold: fold ? [byteAt(fold.from), byteAt(fold.to)] : null}));
        }
    }
    run('Python', '# 字😀\r\ndef f(arg):\r\n  local = arg\r\n  return local\r\n', [
        ['install', {extensions: ['Python']}],
        ['select', {select: 2}],
        ['repeat cached bundle', {extensions: ['Python', 'Python']}],
        ['inactive JS provider reads Python facet', {extensions: ['Python', 'outside', 'JavaScript']}],
        ['edit and remove other providers', {edit: ['local =', 'inner ='], extensions: ['Python']}],
        ['detach', {extensions: ['outside']}],
        ['restore', {extensions: ['Python']}],
    ]);
    run('JavaScript', '// 字😀\r\nfunction f(arg) {\r\n  const local = arg;\r\n  return local;\r\n}\r\n', [
        ['install', {extensions: ['JavaScript']}],
        ['edit before query', {edit: ['字😀', '🧪長文']}],
        ['repeat cached bundle', {extensions: ['JavaScript', 'JavaScript']}],
        ['shared facet', {extensions: ['JavaScript', 'outside', 'TypeScript']}],
        ['TypeScript wins and edits local', {extensions: ['JavaScript', 'outside', 'high TypeScript'], edit: ['local =', 'inner =']}],
        ['restore JavaScript sources and parser', {extensions: ['JavaScript']}],
        ['inactive Python provider reads JS facet', {extensions: ['JavaScript', 'outside', 'Python']}],
        ['detach', {extensions: []}],
    ]);
    run('TypeScript', '// 字😀\r\nfunction f(arg: number) {\r\n  const local: number = arg;\r\n  return local;\r\n}\r\n', [
        ['install', {extensions: ['TypeScript']}],
        ['edit typed declaration', {edit: ['local: number', 'inner: number']}],
        ['select', {select: 3}],
        ['detach', {extensions: []}],
    ]);

    run('JSX', '// 字😀\r\nfunction f(arg) {\r\n  const local = <X />;\r\n  return local;\r\n}\r\n', [
        ['install JSX sources', {extensions: ['JSX']}],
        ['TSX shares metadata and locals', {extensions: ['JSX', 'outside', 'TSX']}],
        ['TSX wins parser selection', {extensions: ['JSX', 'outside', 'high TSX']}],
        ['detach', {extensions: []}],
    ]);

    // The headless handler consumes the caller's default transaction and can
    // dispatch a second transaction. The direct scoped helper builds one edit.
    for (const name of ['JavaScript', 'TypeScript', 'JSX', 'TSX']) {
        const source = 'const view = <X', at = source.length;
        for (const mode of ['plain', 'existing close', 'composition', 'read only']) {
            const state = EditorState.create({doc: source, selection: {anchor: at}, extensions: [extension(name),
                EditorState.readOnly.of(mode === 'read only')]});
            const handlers = state.facet(EditorView.inputHandler);
            const hasHandler = name.endsWith('X');
            assert.deepEqual(handlers, hasHandler ? [js.autoCloseTags.value] : []);
            const view = {state, compositionStarted: mode === 'composition', dispatch(transactions) {
                events = transactions.map(tr => [tr.annotation(Transaction.userEvent), tr.scrollIntoView]);
                view.state = transactions.at(-1).state;
            }};
            let calls = 0, events = [], cached;
            const base = () => cached ??= state.update({
                changes: {from: at, insert: mode === 'existing close' ? '></X>' : '>'},
                selection: {anchor: at + 1}, userEvent: 'input.type', scrollIntoView: true});
            const handled = handlers.some(handler => handler(view, at, at, '>', () => {
                calls++;
                return base();
            }));
            assert.equal(handled, hasHandler && mode === 'plain');
            if (!handled && mode !== 'read only') view.state = base().state;
            console.log(JSON.stringify({kind: 'input', name, mode, source, at, hasHandler, handled, events, calls,
                after: view.state.doc.toString(), head: view.state.selection.main.head}));
        }
    }
    for (const [name, source, typed, mode, beforeTag] of [
        ['JSX', 'const view = <Old', '>', 'rewritten default', 'Old'],
        ['JSX', 'const view = <X', '>', 'caret before input', null],
        ['TSX', 'const view = <Widget><', '/', 'slash', null],
        ['TSX', '// 字😀\r\nconst view: Node = <Widget', '>', 'unicode', null],
    ]) {
        const offsets = projectedOffsets(source);
        const at = offsets.units;
        const state = EditorState.create({doc: source, selection: {anchor: at}, extensions: extension(name)});
        let transactions = [], calls = 0;
        const view = {state, dispatch(batch) { transactions = batch; view.state = batch.at(-1).state; }};
        const insert = beforeTag ? 'New>' : typed;
        const from = beforeTag ? at - beforeTag.length : at;
        const head = mode === 'caret before input' ? at : from + insert.length;
        const base = () => state.update({changes: {from, to: at, insert}, selection: {anchor: head},
            userEvent: 'input.type', scrollIntoView: true});
        const handled = state.facet(EditorView.inputHandler)[0](view, at, at, typed, () => {
            calls++;
            return base();
        });
        assert.equal(handled, mode !== 'caret before input');
        if (!handled) view.state = base().state;
        // Preserve existing line endings in the headless native input.
        const after = source.includes('\r\n') ? view.state.doc.toString().replaceAll('\n', '\r\n')
            : view.state.doc.toString();
        const events = transactions.map(tr => [tr.annotation(Transaction.userEvent), tr.scrollIntoView]);
        console.log(JSON.stringify({kind: 'input', name, mode, source, at: offsets.byteAt(at), typed,
            default: [offsets.byteAt(from), insert, offsets.byteAt(from) + (head - from)],
            hasHandler: true, handled, events, calls, after,
            head: projectedOffsets(after).byteAt(view.state.selection.main.head), assoc: view.state.selection.main.assoc}));
    }
} finally {
    rmSync(root, {recursive: true, force: true});
}
