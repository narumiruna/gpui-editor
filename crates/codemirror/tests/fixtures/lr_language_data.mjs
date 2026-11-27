// Run with Node and a directory of local SHA-512-pinned npm archives.
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {projectedOffsets} from './editor_line_offsets.mjs';
import {pinnedModules} from './pinned_modules.mjs';

assert.equal(process.argv.length, 3, 'provide the directory of local npm archives');
const scratch = mkdtempSync(join(realpathSync(tmpdir()), 'lr-language-data.'));
try {
    const {load} = pinnedModules(resolve(process.argv[2]), scratch);
    const {EditorState, Compartment, Prec} = await load('@codemirror/state');
    const {LanguageDescription, syntaxTree, ensureSyntaxTree, language} = await load('@codemirror/language');
    const {json, jsonLanguage} = await load('@codemirror/lang-json');
    const {lezer, lezerLanguage} = await load('@codemirror/lang-lezer');
    const {markdown} = await load('@codemirror/lang-markdown');
    const {languages} = await load('@codemirror/language-data');
    const emit = row => console.log(JSON.stringify(row));
    const make = (doc, extensions) => EditorState.create({doc, extensions});
    const active = (handle, state, pos) => [-1, 1].map(side => handle.isActiveAt(state, pos, side));
    const ranges = (handle, state, byteAt) => handle.findRegions(state).map(({from, to}) => [byteAt(from), byteAt(to)]);
    const words = (state, pos = 0) => state.languageDataAt('wordChars', pos);
    const description = LanguageDescription.matchLanguageName(languages, 'json5', true);
    assert.equal((await description.load()).language, jsonLanguage);
    assert.equal(LanguageDescription.matchLanguageName(languages, 'lezer', true), null);
    emit({kind: 'catalog', count: languages.length, name: description.name,
        aliases: description.alias, extensions: description.extensions});

    for (const [stock, factory, other, source, mount] of [
        [jsonLanguage, json, lezerLanguage, '{\r\n"😀": [\r0]\r\n}', '> ```json5\r\n> {\r>   "漢":"😀"\r\n> }\r> ```'],
        [lezerLanguage, lezer, jsonLanguage, '// 😀\r\n@top Main {\r Word\r\n}', '> ```lezer\r\n> @top Main {\r>   "😀"\r\n> }\r> ```'],
    ]) {
        const packageName = stock.name;
        const configured = stock.configure({strict: true}, 'renamed');
        const another = configured.configure({}, '');
        assert.equal(configured.data, stock.data);
        assert.equal(another.name, configured.name);
        const data = (word, jsonBrackets, comment) => stock.data.of({wordChars: word, ...(stock === jsonLanguage
            ? {closeBrackets: {brackets: jsonBrackets}} : {commentTokens: {line: comment}})});
        const high = Prec.high(data('@', ['('], '#'));
        const ordinary = data('&', ["'"], '--');
        const low = Prec.low(data('$', ['<'], ';'));
        const wrong = Prec.highest(other.data.of({wordChars: 'wrong'}));
        const {units, byteAt} = projectedOffsets(source);
        const metadata = state => stock === jsonLanguage
            ? state.languageDataAt('closeBrackets', 0).map(d => d.brackets)
            : state.languageDataAt('commentTokens', 0).map(d => [d.line ?? null, d.block ? [d.block.open, d.block.close] : null]);
        for (const [step, extensions] of [
            ['only entries', []], ['stock', [stock]], ['stock factories', [stock, factory(), factory()]],
            ['same parser twice', [configured, Prec.high(configured)]],
            ['distinct same name', [configured, another]], ['stock and renamed', [stock, configured]],
            ['renamed wins', [stock, Prec.high(configured)]], ['other loses', [stock, other]],
            ['other wins', [stock, Prec.high(other)]],
        ]) {
            const state = make(source, [low, ...extensions, ordinary, high, wrong]);
            const at = handle => [active(handle, state, 0), active(handle, state, units)];
            emit({kind: 'install', package: packageName, step, source,
                winner: state.facet(language)?.name ?? null, words: words(state), values: metadata(state),
                active: [at(stock), at(configured)], regions: [ranges(stock, state, byteAt), ranges(configured, state, byteAt)]});
        }
        const leaf = stock.data.of({wordChars: 'one'});
        const same = stock.data.of({wordChars: 'one'});
        const host = Prec.highest(EditorState.languageData.of(() => [{wordChars: 'host'}]));
        emit({kind: 'leaves', package: packageName, words: words(make(source, [host, stock, leaf, Prec.high(leaf), same]))});
        const empty = make('', stock);
        emit({kind: 'empty', package: packageName, active: active(configured, empty, 0), regions: ranges(stock, empty, n => n)});
        const parserSlot = new Compartment();
        const dataSlot = new Compartment();
        const retained = make(source, [parserSlot.of(factory()), dataSlot.of([high, low])]);
        let state = retained;
        for (const [step, effect] of [
            ['factory again', parserSlot.reconfigure(factory())], ['new parser', parserSlot.reconfigure(configured)],
            ['distinct same name', parserSlot.reconfigure(another)],
            ['remove data', dataSlot.reconfigure([])], ['detach', parserSlot.reconfigure([])],
        ]) {
            const before = syntaxTree(state);
            state = state.update({effects: effect}).state;
            emit({kind: 'swap', package: packageName, step, source, reused: before === syntaxTree(state),
                words: words(state), retained: words(retained), regions: ranges(stock, state, byteAt)});
        }

        const body = stock === jsonLanguage ? '{}' : 'R { a }';
        const resolve = {codeLanguages: () => stock};
        for (const [mode, config, source] of [
            ['plain', {}, mount], ['catalog', {codeLanguages: languages}, mount], ['resolver', resolve, mount],
            ['empty', resolve, '```' + packageName + '\r\n```'],
            ['unclosed', resolve, '```' + packageName + '\r\n' + body],
            ['indented', {defaultCodeLanguage: stock}, '    ' + body + '\r\n'],
        ]) {
            let state = make(source, [markdown(config), low, high]);
            assert(ensureSyntaxTree(state, state.doc.length, 1000));
            state = state.update({}).state;
            const map = projectedOffsets(source);
            const regions = ranges(stock, state, map.byteAt);
            const boundaries = new Map();
            for (let pos = 0; pos <= map.units; pos++) {
                try { boundaries.set(map.byteAt(pos), pos); } catch { /* Surrogate interior. */ }
            }
            const positions = [...new Set([0, ...regions.flatMap(([from, to]) => [from - 1, from, from + 1, to - 1, to, to + 1])])].sort((a, b) => a - b);
            const probes = positions.filter(at => boundaries.has(at)).map(at => {
                const pos = boundaries.get(at);
                return [at, active(configured, state, pos), ...[-1, 1].map(side => state.languageDataAt('wordChars', pos, side))];
            });
            emit({kind: 'mount', package: packageName, mode, source, regions, probes});
        }
    }
} finally {
    rmSync(scratch, {recursive: true, force: true});
}
