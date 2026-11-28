// node crates/codemirror/tests/fixtures/language_data_html.mjs <archive-root> [--emit]
import assert from 'node:assert/strict';
import {mkdtempSync, readFileSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {projectedOffsets, projectInput} from './editor_line_offsets.mjs';
import {pinnedModules} from './pinned_modules.mjs';

assert(process.argv.length === 3 || process.argv.length === 4 && process.argv[3] === '--emit');
const root = mkdtempSync(join(realpathSync(tmpdir()), 'language-data-html-'));
try {
    const {load} = pinnedModules(resolve(process.argv[2]), root);
    const {languages} = await load('@codemirror/language-data');
    const {html, htmlLanguage, autoCloseTags} = await load('@codemirror/lang-html');
    const {javascriptLanguage, localCompletionSource} = await load('@codemirror/lang-javascript');
    const {cssLanguage, cssCompletionSource} = await load('@codemirror/lang-css');
    const {CompletionContext} = await load('@codemirror/autocomplete');
    const {Compartment, EditorSelection, EditorState, Prec, StateEffect, Transaction} = await load('@codemirror/state');
    const {EditorView} = await load('@codemirror/view');
    const {language, languageDataProp, syntaxTree} = await load('@codemirror/language');
    const bundles = new Map(await Promise.all(['HTML', 'JavaScript', 'CSS'].map(async name => {
        const entry = languages.find(entry => entry.name === name);
        const bundle = await entry.load();
        assert.equal(await entry.load(), bundle);
        return [name, bundle];
    })));
    const bundle = bundles.get('HTML'), standaloneJs = bundles.get('JavaScript').support;
    const standaloneCss = bundles.get('CSS').support;
    const [schema, handler, [fixed, local, jsxHandler], css] = bundle.support;
    assert.equal(bundle.language, htmlLanguage);
    assert.equal(htmlLanguage.parser.topNode.name, 'Document');
    assert.equal(htmlLanguage.parser.topNode.prop(languageDataProp), htmlLanguage.data);
    assert.deepEqual(bundle.extension, [htmlLanguage, bundle.support]);
    assert.deepEqual(bundle.support, [schema, autoCloseTags, [fixed, local, []], css]);
    assert.equal(handler.facet, EditorView.inputHandler);
    assert.deepEqual(jsxHandler, []);
    assert.equal(schema.facet, htmlLanguage.data);
    assert.equal(fixed.facet, javascriptLanguage.data);
    assert.equal(local.facet, javascriptLanguage.data);
    assert.equal(css.facet, cssLanguage.data);
    assert.notEqual(schema.value.autocomplete, html().support[0].value.autocomplete);
    assert.notEqual(fixed.value.autocomplete, standaloneJs[0].value.autocomplete);
    assert.equal(local.value.autocomplete, localCompletionSource);
    assert.equal(css.value.autocomplete, cssCompletionSource);
    assert.notEqual(local, standaloneJs[1]);
    assert.notEqual(css, standaloneCss);
    const bare = EditorState.create({doc: '<script>abc</script>', extensions: htmlLanguage.extension});
    assert(htmlLanguage.isActiveAt(bare, 1));
    assert(javascriptLanguage.isActiveAt(bare, 8));
    assert.deepEqual(bare.languageDataAt('autocomplete', 1), []);
    assert.deepEqual(bare.languageDataAt('autocomplete', 8), []);
    const ids = new Map([[schema.value.autocomplete, 'html'], [fixed.value.autocomplete, 'fixed'],
        [localCompletionSource, 'local'], [cssCompletionSource, 'css'], [standaloneJs[0].value.autocomplete, 'js-fixed']]);
    const keys = ['zIndex', 'display', 'backgroundColor', 'cssFloat', 'cssText'];
    globalThis.document = {body: {style: Object.fromEntries(keys.map(key => [key, '']))}};
    const rows = [{keys}];
    const extension = name => name.startsWith('high ') ? Prec.high(extension(name.slice(5))) : bundles.get(name).extension;
    const sideData = (state, at, side) => ({
        sources: state.languageDataAt('autocomplete', at, side).map(fn => {
            assert(ids.has(fn));
            return ids.get(fn);
        }),
        comments: state.languageDataAt('commentTokens', at, side).map(({line, block}) => [line ?? null, block ? [block.open, block.close] : null]),
        words: state.languageDataAt('wordChars', at, side),
        brackets: state.languageDataAt('closeBrackets', at, side).map(({brackets, before, stringPrefixes}) => [brackets ?? null, before ?? null, stringPrefixes ?? []]),
    });
    function summarize(result, byteAt) {
        if (!result) return null;
        let hash = 2166136261;
        for (const {label, type, detail, boost, apply, ...rest} of result.options) {
            assert.deepEqual(rest, {});
            const text = typeof apply === 'function' ? '@snippet' : apply ?? '';
            for (const byte of Buffer.from(`${label}\0${type ?? ''}\0${detail ?? ''}\0${boost ?? 0}\0${text}\n`))
                hash = Math.imul(hash ^ byte, 16777619) >>> 0;
        }
        return [byteAt(result.from), result.to == null ? null : byteAt(result.to), result.filter ?? true,
            result.options.length, hash];
    }
    function runState(name, source, points, steps = [['install', {extensions: ['HTML']}]]) {
        rows.push({kind: 'start', name, source});
        const slot = new Compartment();
        let state = EditorState.create({doc: source, extensions: slot.of([])});
        for (const [step, {extensions, edit, select}] of steps) {
            const before = syntaxTree(state), {byteAt: oldByteAt} = projectedOffsets(source);
            const from = edit && state.doc.toString().indexOf(edit[0]), to = edit && from + edit[0].length;
            assert(!edit || from >= 0);
            const tr = state.update({changes: edit && {from, to, insert: edit[1]},
                selection: select === undefined ? undefined : {anchor: select},
                effects: extensions && slot.reconfigure(extensions.map(extension))});
            source = projectInput(source, tr);
            state = tr.state;
            const {byteAt} = projectedOffsets(source), tree = syntaxTree(state);
            const active = [...bundles].find(([, bundle]) => bundle.language === state.facet(language))?.[0] ?? null;
            const queries = points(state.doc.toString()).map(([label, at]) => ({label, at: byteAt(at),
                data: [-1, 1].map(side => sideData(state, at, side)),
                results: [false, true].map(explicit => state.languageDataAt('autocomplete', at, -1).map(fn => {
                    // Start each source from the tree root. A cached host node
                    // from a previous query can hide a quoted overlay.
                    tree.resolveInner(0, -1);
                    return summarize(fn(new CompletionContext(state, at, explicit)), byteAt);
                })),
            }));
            const handlers = state.facet(EditorView.inputHandler);
            assert(handlers.every(fn => fn === autoCloseTags.value));
            rows.push({kind: 'state', name, step, source, active, root: tree.topNode.name, reused: !!active && before === tree,
                extensions, edit: edit && [oldByteAt(from), oldByteAt(to), edit[1]], select: select === undefined ? undefined : oldByteAt(select),
                head: byteAt(state.selection.main.head), handlers: handlers.length, queries});
        }
    }
    const ends = (label, text, word) => {
        const at = text.indexOf(word);
        assert(at >= 0);
        return [[`${label} start`, at], [`${label} inner`, at + 1], [`${label} end`, at + word.length]];
    };
    runState('composition', '<!-- 字😀 -->\r\n<div class="name">\r\n<script lang="ts">const local: number = 1; loc;</script>\r\n<style>.card { bac }</style>\r\n</div>', text => [
        ['schema', text.indexOf('class') + 3], ['TS locals', text.indexOf('loc;') + 3],
        ...ends('CSS property', text, 'bac'),
    ], [
        ['install', {extensions: ['HTML']}], ['selection only', {select: 3}],
        ['repeat cached bundle', {extensions: ['HTML', 'HTML']}],
        ['edit host prefix', {edit: ['字😀', '🧪長文']}],
        ['edit mounted declaration', {edit: ['local:', 'other:']}],
        ['distinct child facet inputs', {extensions: ['HTML', 'JavaScript', 'CSS']}],
        ['child root wins only when explicit', {extensions: ['high JavaScript', 'HTML']}],
        ['detach', {extensions: []}], ['restore', {extensions: ['HTML']}],
    ]);
    for (const [name, source, word] of [
        ['script endpoints', '<script>co</script>', 'co'],
        ['style body endpoints', '<style>co</style>', 'co'],
        ['quoted Styles', '<div style="color: red">', 'color: red'],
        ['unquoted Styles', '<div style=co>', 'co'],
        ['quoted event', '<div onclick="return foo">', 'return foo'],
        ['unquoted event', '<div onclick=foo>', 'foo'],
        ['JSX slot one', '<script type="text/jsx">const x = <Foo/>;</script>', 'Foo'],
        ['TSX slot one', '<script type="text/typescript-jsx">const x = <Foo/>;</script>', 'Foo'],
        ['import map', '<script type="importmap">{"imports":{}}</script>', 'imports'],
        ['unknown script type', '<script type="text/plain">words</script>', 'words'],
        ['textarea', '<textarea>words</textarea>', 'words'],
        ['empty quoted Styles', '<div style="">', null],
        ['empty raw script', '<script></script>', null],
        ['open raw script at EOF', '<script>co', 'co'],
        ['malformed quoted Styles', '<div style="co', 'co'],
    ]) runState(name, source, text => word ? ends(name, text, word)
        : [[name, text.indexOf(name === 'empty raw script' ? '><' : '""') + 1]]);

    const marker = StateEffect.define({map: (pos, changes) => changes.mapPos(pos, 1)});
    const ranges = (state, byteAt) => state.selection.ranges.map(r => [byteAt(r.anchor), byteAt(r.head), r.assoc]);
    function describe(tr, source) {
        const {byteAt} = projectedOffsets(source), after = projectInput(source, tr);
        const newOffset = projectedOffsets(after).byteAt, changes = [];
        tr.changes.iterChanges((from, to, _fromB, _toB, insert) => changes.push([byteAt(from), byteAt(to), insert.toString()]));
        return {changes, after, ranges: ranges(tr.state, newOffset), main: tr.state.selection.mainIndex,
            event: tr.annotation(Transaction.userEvent), scroll: tr.scrollIntoView,
            markers: tr.effects.filter(effect => effect.is(marker)).map(effect => newOffset(effect.value))};
    }
    function runInput(name, marked, typed, mode = 'normal') {
        const parts = marked.split('|'), source = parts.join(''), {byteAt} = projectedOffsets(source);
        let pos = 0;
        const carets = parts.slice(0, -1).map(part => pos += projectedOffsets(part).units);
        const state = EditorState.create({doc: source, selection: EditorSelection.create(carets.map(at => EditorSelection.cursor(at))),
            extensions: [bundle.extension, bundle.extension, EditorState.allowMultipleSelections.of(true),
                EditorState.readOnly.of(mode === 'read only'), EditorState.transactionFilter.of(tr => {
                    if (!tr.isUserEvent('input.type')) return tr;
                    if (mode === 'rewrite') return {changes: {from: 1, to: carets[0], insert: 'rewritten>'},
                        selection: {anchor: 11}, userEvent: 'input.type', scrollIntoView: true};
                    if (mode === 'omit typed') return {changes: {from: carets[0], insert: 'x'},
                        selection: {anchor: carets[0] + 1}, userEvent: 'input.type', scrollIntoView: true};
                    return tr;
                })]});
        const at = carets[0];
        let steps = [], calls = 0, cached;
        const view = {state, composing: mode === 'composition', dispatch(batch) {
            steps = batch;
            view.state = batch.at(-1).state;
        }};
        const spec = {...state.replaceSelection(typed), userEvent: 'input.type', scrollIntoView: true};
        if (mode === 'marker') spec.effects = [marker.of(at + 1)];
        const base = () => cached ??= state.update(spec);
        const handlers = state.facet(EditorView.inputHandler);
        assert.deepEqual(handlers, [autoCloseTags.value]);
        const handled = handlers[0](view, at, at, typed, () => { calls++; return base(); });
        if (!handled && !view.composing && !state.readOnly) view.dispatch([base()]);
        let after = source;
        const transactions = steps.map(tr => {
            const row = describe(tr, after);
            after = row.after;
            return row;
        });
        const markers = steps.flatMap((tr, i) => tr.effects.filter(effect => effect.is(marker)).map(effect => {
            let pos = effect.value;
            for (const later of steps.slice(i + 1)) pos = later.changes.mapPos(pos, 1);
            return projectedOffsets(after).byteAt(pos);
        }));
        rows.push({kind: 'input', name, source, carets: carets.map(byteAt), typed, mode, calls, handled, transactions, markers});
    }
    runInput('accept a default effect', '😀\r\n<section|', '>', 'marker');
    runInput('default filter rewrites tag name', '<old|', '>', 'rewrite');
    runInput('default filter removes typed token', '<old|', '>', 'omit typed');
    runInput('slash association and adjacent angle', '<section><|>after', '/');
    runInput('duplicate close', '<section|</section>', '>');
    runInput('void tag', '<br|', '>');
    runInput('unfinished raw script', '<script|', '>');
    runInput('unfinished raw style', '<style|', '>');
    runInput('unsupported script type stays HTML', '<script type="text/plain"|', '>');
    runInput('secondary raw tag uses post-input syntax', '<div|\r\n<script|', '>');
    runInput('script body endpoint', '<script>|</script>', '>');
    runInput('quoted style endpoint', '<div style="co|">', '>');
    runInput('no JSX handler in HTML', '<script type="text/jsx">const x = <Foo|;\r\n</script>', '>');
    runInput('composition gate', '<div|', '>', 'composition');
    runInput('read-only gate', '<div|', '>', 'read only');
    const generated = `${rows.map(row => JSON.stringify(row)).join('\n')}\n`;
    if (process.argv[3] === '--emit') process.stdout.write(generated);
    else assert.equal(generated, readFileSync(new URL('./language_data_html.jsonl', import.meta.url), 'utf8'));
} finally {
    rmSync(root, {recursive: true, force: true});
}
