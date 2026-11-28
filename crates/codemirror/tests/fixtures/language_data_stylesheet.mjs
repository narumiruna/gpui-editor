// node crates/codemirror/tests/fixtures/language_data_stylesheet.mjs <archive-root> [--emit]
import assert from 'node:assert/strict';
import {mkdtempSync, readFileSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {projectedOffsets} from './editor_line_offsets.mjs';
import {pinnedModules} from './pinned_modules.mjs';

assert(process.argv.length === 3 || process.argv.length === 4 && process.argv[3] === '--emit');
const root = mkdtempSync(join(realpathSync(tmpdir()), 'language-data-stylesheet-'));
try {
    const {load} = pinnedModules(resolve(process.argv[2]), root);
    const {languages} = await load('@codemirror/language-data');
    const {css, cssLanguage, cssCompletionSource} = await load('@codemirror/lang-css');
    const {less, lessLanguage, lessCompletionSource} = await load('@codemirror/lang-less');
    const {sass, sassLanguage, sassCompletionSource} = await load('@codemirror/lang-sass');
    const {jsonLanguage} = await load('@codemirror/lang-json');
    const {Compartment, EditorState, Prec} = await load('@codemirror/state');
    const {foldable, getIndentation, language, languageDataProp, syntaxTree} = await load('@codemirror/language');
    const {CompletionContext} = await load('@codemirror/autocomplete');
    const cases = [
        ['CSS', css, cssLanguage, cssCompletionSource, 'css', 'LESS'],
        ['LESS', less, lessLanguage, lessCompletionSource, 'less', 'CSS'],
        ['Sass', () => sass({indented: true}), null, sassCompletionSource, 'sass', 'SCSS'],
        ['SCSS', sass, sassLanguage, sassCompletionSource, 'sass', 'Sass'],
    ];
    const bundles = new Map(), identities = new Map(), parserNames = new Map();
    const outside = () => null;
    identities.set(outside, 'outside');
    for (const [name, factory, exported, autocomplete, sourceId] of cases) {
        const entry = languages.find(entry => entry.name === name);
        const bundle = await entry.load();
        const stock = bundle.language, fresh = factory();
        assert.equal(await entry.load(), bundle);
        assert.equal(stock, exported ?? fresh.language);
        assert.equal(stock, fresh.language);
        assert.equal(stock.name, sourceId);
        assert.equal(stock.parser.topNode.name, 'StyleSheet');
        assert.equal(stock.parser.topNode.prop(languageDataProp), stock.data);
        assert.notEqual(bundle, fresh);
        assert.deepEqual(bundle.extension, [stock, bundle.support]);
        assert.equal(stock.extension.length, 2);
        assert.equal(stock.extension[0].facet, language);
        assert.equal(stock.extension[1].facet, EditorState.languageData);
        assert.equal(bundle.support.facet, stock.data);
        assert.deepEqual(bundle.support.value, {autocomplete});
        assert.notEqual(bundle.support, fresh.support);
        assert.deepEqual(fresh.support.value, bundle.support.value);
        assert(!parserNames.has(stock));
        assert(![...parserNames.keys()].some(other => stock.parser === other.parser));
        parserNames.set(stock, name);
        if (identities.has(autocomplete)) assert.equal(identities.get(autocomplete), sourceId);
        identities.set(autocomplete, sourceId);
        const bare = EditorState.create({extensions: stock.extension});
        assert.deepEqual(bare.languageDataAt('autocomplete', 0), []);
        assert.equal(stock.isActiveAt(bare, 0), true);
        bundles.set(name, bundle);
    }
    const facet = name => bundles.get(name).language.data;
    assert.equal(facet('Sass'), facet('SCSS'));
    assert.equal(new Set(cases.map(([name]) => facet(name))).size, 3);
    assert.equal(identities.size, 4);
    parserNames.set(jsonLanguage, 'JSON');

    const sassSlot = new Compartment();
    const indented = EditorState.create({extensions: sassSlot.of(bundles.get('Sass').extension)});
    const braced = indented.update({effects: sassSlot.reconfigure(bundles.get('SCSS').extension)}).state;
    assert.equal(indented.doc, braced.doc);
    assert.notEqual(syntaxTree(indented), syntaxTree(braced));
    assert.equal(indented.languageDataAt('autocomplete', 0)[0], braced.languageDataAt('autocomplete', 0)[0]);

    const keys = ['zIndex', 'display', 'backgroundColor', 'cssFloat', 'cssText'];
    globalThis.document = {body: {style: Object.fromEntries(keys.map(key => [key, '']))}};
    const rows = [{keys}];
    const markers = ['CSS', 'LESS', 'Sass'].map(name => facet(name).of({catalogProbe: name}));
    const extension = name => {
        if (name === 'outside') return EditorState.languageData.of(() => [{autocomplete: outside, catalogProbe: name, wordChars: '!'}]);
        if (name === 'JSON') return jsonLanguage.extension;
        if (name.startsWith('high ')) return Prec.high(extension(name.slice(5)));
        return bundles.get(name).extension;
    };
    function summarize(result, byteAt) {
        if (!result) return null;
        let hash = 2166136261;
        for (const {label, type, apply, ...rest} of result.options) {
            assert.deepEqual(rest, {});
            for (const byte of Buffer.from(`${label}\0${type}\0${apply ?? ''}\n`))
                hash = Math.imul(hash ^ byte, 16777619) >>> 0;
        }
        assert.equal(result.to, undefined);
        assert.equal(result.filter, undefined);
        assert(result.validFor instanceof RegExp);
        return [byteAt(result.from), result.options.length, hash];
    }
    function body(name) {
        return name === 'Sass' ? '.card\r\n  color: red\r\n  width: auto\r\n'
            : '.card {\r\n  color: red;\r\n  width: auto;\r\n}\r\n';
    }
    for (const [name, , , , sourceId, alternate] of cases) {
        let source = `/* 字😀 */\r\n${body(name)}`;
        rows.push({name, source});
        const slot = new Compartment();
        let state = EditorState.create({doc: source, extensions: [markers, slot.of([])]});
        const steps = [
            ['install', {extensions: [name]}],
            ['select', {select: 3}],
            ['edit', {edit: ['字😀', '🧪長文']}],
            ['repeat bundle', {extensions: [name, name]}],
            ['shared bundle at higher precedence', {extensions: [name, 'outside', `high ${name}`]}],
            ['inactive alternate provider', {extensions: [name, 'outside', alternate]}],
            ['outside first', {extensions: ['outside', name, alternate]}],
            ['alternate wins', {extensions: [name, 'outside', `high ${alternate}`],
                replaceBody: body(name) !== body(alternate) ? alternate : undefined}],
            ['JSON wins', {extensions: [name, 'outside', alternate, 'high JSON']}],
            ['restore', {extensions: [name], replaceBody: body(name) !== body(alternate) ? name : undefined}],
            ['detach', {extensions: ['outside']}],
            ['remove outside', {extensions: []}],
        ];
        for (const [step, {extensions, edit, select, replaceBody}] of steps) {
            const before = syntaxTree(state), {byteAt: oldByteAt} = projectedOffsets(source);
            const oldText = state.doc.toString();
            const change = edit ? {from: oldText.indexOf(edit[0]), to: oldText.indexOf(edit[0]) + edit[0].length, insert: edit[1]}
                : replaceBody ? {from: oldText.indexOf('.card'), to: oldText.length, insert: body(replaceBody)} : undefined;
            assert(!change || change.from >= 0);
            const tr = state.update({changes: change,
                selection: select === undefined ? undefined : {anchor: select},
                effects: extensions && slot.reconfigure(extensions.map(extension))});
            if (change) {
                const raw = Buffer.from(source);
                source = Buffer.concat([raw.subarray(0, oldByteAt(change.from)), Buffer.from(change.insert),
                    raw.subarray(oldByteAt(change.to))]).toString();
            }
            state = tr.state;
            const text = state.doc.toString(), {byteAt} = projectedOffsets(source);
            assert.equal(source.replace(/\r\n?/g, '\n'), text);
            const tree = syntaxTree(state), parser = parserNames.get(state.facet(language)) ?? null;
            const at = text.indexOf('color') + 3;
            const data = key => state.languageDataAt(key, at);
            const activeFacets = cases.map(([key]) => key).filter(key => bundles.get(key).language.isActiveAt(state, at));
            const stylesheet = activeFacets.length !== 0;
            const line = state.doc.line(2), indentAt = state.doc.line(3).from;
            const fold = stylesheet ? foldable(state, line.from, line.to) : null;
            const services = stylesheet ? {indentAt: byteAt(indentAt), indent: getIndentation(state, indentAt),
                line: [byteAt(line.from), byteAt(line.to)], fold: fold ? [byteAt(fold.from), byteAt(fold.to)] : null} : null;
            const queries = [at, text.indexOf('red') + 2, 3];
            const completions = queries.map(pos => {
                const callbacks = state.languageDataAt('autocomplete', pos, -1);
                assert.deepEqual(state.languageDataAt('autocomplete', pos, 1), callbacks);
                return {at: byteAt(pos), sources: callbacks.map(callback => {
                    assert(identities.has(callback));
                    const run = explicit => summarize(callback(new CompletionContext(state, pos, explicit)), byteAt);
                    return {id: identities.get(callback), implicit: run(false), explicit: run(true)};
                })};
            });
            if (step === 'install') {
                const [source] = data('autocomplete');
                assert.equal(source, bundles.get(name).support.value.autocomplete);
                assert.equal(identities.get(source), sourceId);
                const result = source(new CompletionContext(state, at, true));
                assert.deepEqual(result.options.map(option => option.label), ['background-color', 'display', 'z-index']);
            }
            rows.push({step, extensions, edit: change && [oldByteAt(change.from), oldByteAt(change.to), change.insert],
                select: select === undefined ? undefined : oldByteAt(select), head: byteAt(state.selection.main.head),
                parser, root: tree.topNode.name, activeFacets, reused: before === tree,
                comments: data('commentTokens').map(({line, block}) => [line ?? null, [block.open, block.close]]),
                brackets: data('closeBrackets').map(({brackets, before, stringPrefixes}) => [brackets ?? null, before ?? null, stringPrefixes ?? []]),
                wordChars: data('wordChars'), indentOnInput: data('indentOnInput').map(re => ['}', '  }', 'x}', '  ]'].map(value => re.test(value))),
                markers: data('catalogProbe'), completions, services});
        }
    }
    const actual = rows.map(row => JSON.stringify(row)).join('\n') + '\n';
    if (process.argv.includes('--emit')) process.stdout.write(actual);
    else assert.equal(actual, readFileSync(new URL('./language_data_stylesheet.jsonl', import.meta.url), 'utf8'));
} finally {
    delete globalThis.document;
    rmSync(root, {recursive: true, force: true});
}
