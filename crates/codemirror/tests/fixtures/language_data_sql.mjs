// node crates/codemirror/tests/fixtures/language_data_sql.mjs <archive-root>
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {projectedOffsets, projectInput} from './editor_line_offsets.mjs';
import {pinnedModules} from './pinned_modules.mjs';

assert.equal(process.argv.length, 3);
const root = mkdtempSync(join(realpathSync(tmpdir()), 'language-data-sql-'));
try {
    const {load} = pinnedModules(resolve(process.argv[2]), root);
    const {languages} = await load('@codemirror/language-data');
    const sql = await load('@codemirror/lang-sql');
    const {Compartment, EditorState, Prec} = await load('@codemirror/state');
    const {foldable, getIndentation, language, languageDataProp, syntaxTree} = await load('@codemirror/language');
    const {CompletionContext} = await load('@codemirror/autocomplete');
    const {jsonLanguage} = await load('@codemirror/lang-json');
    const cases = [
        ['CQL', 'Cassandra', 'frozen, Infinity'],
        ['MariaDB SQL', 'MariaSQL', 'groupby_concat, `x`'],
        ['MS SQL', 'MSSQL', 'top, [a b]'],
        ['MySQL', 'MySQL', 'longblob, 0b010, "x"'],
        ['PLSQL', 'PLSQL', "rownum, q'[x]'"],
        ['PostgreSQL', 'PostgreSQL', 'ILIKE, $tag$x$tag$'],
        ['SQL', 'StandardSQL', 'distinct, "x"'],
        ['SQLite', 'SQLite', 'pragma, ?123'],
    ];
    const bundles = new Map(), dialects = new Map(), identities = new Map();
    const outside = () => null;
    identities.set(outside, 'outside');
    for (const [name, exportName] of cases) {
        const entry = languages.find(entry => entry.name === name);
        const dialect = sql[exportName];
        const bundle = await entry.load();
        assert.equal(await entry.load(), bundle);
        assert.equal(bundle.language, dialect.language);
        assert.equal(bundle.language.name, 'sql');
        assert.equal(bundle.language.parser.topNode.name, 'Script');
        assert.equal(bundle.language.parser.topNode.prop(languageDataProp), bundle.language.data);
        assert.deepEqual(bundle.extension, [bundle.language, bundle.support]);
        assert.deepEqual(bundle.support[0], []);
        assert.equal(bundle.support.length, 2);
        const source = bundle.support[1].value.autocomplete;
        assert.equal(bundle.support[1].facet, bundle.language.data);
        assert.deepEqual(Object.keys(bundle.support[1].value), ['autocomplete']);
        assert(!identities.has(source));
        assert(![...dialects.values()].some(other => other.language.data === bundle.language.data));
        const fresh = sql.sql({dialect});
        assert.equal(fresh.language, bundle.language);
        assert.notEqual(fresh.support[1].value.autocomplete, source);
        const custom = sql.sql({dialect, schema: {probe_table: ['probe_column']}, upperCaseKeywords: true,
            keywordCompletion: (label, type) => ({label, type, boost: 7})});
        assert.equal(custom.language, bundle.language);
        assert.equal(custom.support[0].facet, bundle.language.data);
        assert.equal(custom.support[1].facet, bundle.language.data);
        const state = EditorState.create({extensions: custom.extension});
        const sources = state.languageDataAt('autocomplete', 0);
        assert.deepEqual(sources, custom.support.map(input => input.value.autocomplete));
        const run = callback => callback(new CompletionContext(state, 0, true));
        assert.equal(run(sources[0]).options[0].label, 'probe_table');
        assert(run(sources[1]).options.some(option => option.label === 'SELECT' && option.boost === 7));
        const bare = EditorState.create({extensions: dialect.extension});
        assert.deepEqual(bare.languageDataAt('autocomplete', 0), []);
        bundles.set(name, bundle);
        dialects.set(name, dialect);
        identities.set(source, name);
    }

    const facetMarkers = [...dialects].map(([name, dialect]) => dialect.language.data.of({catalogProbe: name}));
    const extension = name => {
        if (name === 'outside') return EditorState.languageData.of(() => [{autocomplete: outside, catalogProbe: name}]);
        if (name === 'JSON') return jsonLanguage.extension;
        if (name.startsWith('high ')) return Prec.high(extension(name.slice(5)));
        return bundles.get(name).extension;
    };
    function summarize(result, byteAt) {
        if (!result) return null;
        let hash = 0xcbf29ce484222325n;
        for (const {label, type, boost, ...rest} of result.options) {
            assert.deepEqual(rest, {});
            const bytes = Buffer.from(`${label}\0${type}\0${boost}\n`);
            for (const byte of bytes) hash = BigInt.asUintN(64, (hash ^ BigInt(byte)) * 0x100000001b3n);
        }
        assert(result.validFor instanceof RegExp);
        return {from: byteAt(result.from), to: result.to == null ? null : byteAt(result.to),
            filter: result.filter ?? true, count: result.options.length, hash: hash.toString(16).padStart(16, '0')};
    }
    for (const [name, exportName, sample] of cases) {
        let source = `-- 字😀\r\nSELECT ${sample};\r\nSELECT foo.bar,\r\n 'x', se`;
        console.log(JSON.stringify({name, exportName, source}));
        const slot = new Compartment();
        let state = EditorState.create({doc: source, extensions: [facetMarkers, slot.of([])]});
        const alternate = name === 'SQL' ? 'PostgreSQL' : 'SQL';
        const steps = [
            ['install', {extensions: [name]}],
            ['select', {select: 3}],
            ['edit', {edit: ['字😀', '🧪長文']}],
            ['repeat bundle', {extensions: [name, name]}],
            ['inactive alternate provider', {extensions: [name, 'outside', alternate]}],
            ['alternate wins and edit', {extensions: [name, 'outside', `high ${alternate}`], edit: ['se', 'select']}],
            ['JSON wins', {extensions: [name, 'outside', 'high JSON']}],
            ['restore', {extensions: [name]}],
            ['detach', {extensions: ['outside']}],
        ];
        for (const [step, {extensions, edit, select}] of steps) {
            const before = syntaxTree(state), {byteAt: oldByteAt} = projectedOffsets(source);
            const from = edit && state.doc.toString().lastIndexOf(edit[0]);
            assert(!edit || from >= 0);
            const to = edit && from + edit[0].length;
            const tr = state.update({
                changes: edit && {from, to, insert: edit[1]},
                selection: select === undefined ? undefined : {anchor: select},
                effects: extensions && slot.reconfigure(extensions.map(extension)),
            });
            source = projectInput(source, tr);
            state = tr.state;
            const text = state.doc.toString(), {byteAt} = projectedOffsets(source);
            const next = syntaxTree(state), top = next.topNode;
            const parser = state.facet(language);
            const active = [...dialects].filter(([, dialect]) => dialect.language.data === top.type.prop(languageDataProp))
                .map(([name]) => name);
            assert.deepEqual(active, [...dialects].filter(([, dialect]) => dialect.language.isActiveAt(state, text.length))
                .map(([name]) => name));
            const at = text.length, data = key => state.languageDataAt(key, at);
            const queries = step === 'install' ? [at, text.indexOf('foo.') + 4, text.lastIndexOf("'x'") + 2, 3,
                text.lastIndexOf('SELECT') + 7] : [at];
            const completions = queries.map(pos => ({at: byteAt(pos), sources: state.languageDataAt('autocomplete', pos).map(callback => {
                assert(identities.has(callback));
                const run = explicit => summarize(callback(new CompletionContext(state, pos, explicit)), byteAt);
                return {id: identities.get(callback), implicit: run(false), explicit: run(true)};
            })}));
            const statement = active.length ? top.getChild('Statement') : null;
            const firstStatement = statement ? [] : null;
            for (let node = statement?.firstChild; node; node = node.nextSibling)
                firstStatement.push([node.name, byteAt(node.from), byteAt(node.to)]);
            const line = state.doc.line(3), continued = state.doc.line(4).from;
            const fold = active.length ? foldable(state, line.from, line.to) : null;
            const services = active.length ? {indentAt: byteAt(continued), indent: getIndentation(state, continued),
                line: [byteAt(line.from), byteAt(line.to)], fold: fold ? [byteAt(fold.from), byteAt(fold.to)] : null} : null;
            console.log(JSON.stringify({step, extensions, edit: edit && [oldByteAt(from), oldByteAt(to), edit[1]],
                select: select === undefined ? undefined : oldByteAt(select), head: byteAt(state.selection.main.head),
                parser: parser?.name ?? null, root: top.name, active, reused: before === next,
                comments: data('commentTokens').map(({line, block}) => [line ?? null, block ? [block.open, block.close] : null]),
                brackets: data('closeBrackets').map(({brackets, before, stringPrefixes}) => [brackets ?? null, before ?? null, stringPrefixes ?? []]),
                wordChars: data('wordChars'), indentOnInput: data('indentOnInput').length, markers: data('catalogProbe'),
                firstStatement, completions, services}));
        }
    }
} finally {
    rmSync(root, {recursive: true, force: true});
}
