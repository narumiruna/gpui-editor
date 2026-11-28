// node crates/codemirror/tests/fixtures/language_data_catalog_load.mjs <archive-root>
import assert from 'node:assert/strict';
import {mkdtempSync, readFileSync, realpathSync, rmSync} from 'node:fs';
import {createRequire} from 'node:module';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {runInNewContext} from 'node:vm';
import {pinnedModules} from './pinned_modules.mjs';

const catalogPackage = '@codemirror/language-data';
const ownerPackage = '@codemirror/language';
const oracle = JSON.parse(readFileSync(new URL('../../src/language_data/oracle.json', import.meta.url)));
const id = (catalog, match) => match === null ? null : catalog.indexOf(match);
const metadata = d => [d.name, d.alias, d.extensions,
    d.filename ? [d.filename.source, d.filename.flags] : null];
const sharedRecipes = oracle.catalog.flatMap((row, right) => oracle.catalog.slice(0, right)
    .flatMap((prior, left) => JSON.stringify(prior[4]) === JSON.stringify(row[4]) ? [[left, right]] : []));

// Intercept only the dynamic target imports and composition calls. A direct
// export, a factory call, and an exported dialect passed as an option stay distinct.
async function assertRecipes(source, owner, commonJs) {
    const encode = value => {
        if (typeof value === 'function') return value.exportId;
        if (Array.isArray(value)) return value.map(encode);
        if (value && typeof value === 'object') {
            return Object.fromEntries(Object.entries(value).map(([key, val]) => [key, encode(val)]));
        }
        return value;
    };
    class Stream {
        constructor(mode) { this.mode = mode; }
    }
    class Support {
        constructor(stream) {
            assert.ok(stream instanceof Stream);
            this.recipe = encode(stream.mode);
        }
    }
    const language = {
        LanguageDescription: owner.LanguageDescription,
        StreamLanguage: {define: mode => new Stream(mode)},
        LanguageSupport: Support,
    };
    const body = commonJs
        ? source.replace(/^var language = require\('@codemirror\/language'\);$/m, '')
            .replace(/^exports\.languages = languages;$/m, 'languages;')
        : source.replace(/^import .* from '@codemirror\/language';$/m, '')
            .replace(/^export \{ languages \};$/m, 'languages;');
    const catalog = runInNewContext(body.replaceAll('import(', 'recordImport('), {
        ...language, language,
        recordImport: async module => new Proxy({}, {
            get(_, name) {
                if (name === 'then') return undefined;
                const call = (...args) => [module, name, encode(args)];
                call.exportId = [module, name, null];
                return call;
            },
        }),
    });
    for (const [i, description] of catalog.entries()) {
        const loaded = await description.load();
        const expected = oracle.catalog[i][4];
        assert.equal(loaded instanceof Support, expected[0].startsWith('@codemirror/legacy-modes/'),
            description.name);
        assert.equal(JSON.stringify(loaded.recipe ?? loaded), JSON.stringify(expected), description.name);
    }
    assert.equal(catalog.length, oracle.catalog.length);
}

async function capture(namespace, owner, esmOwner, statePackage, esmState) {
    assert.deepEqual(Object.keys(namespace), ['languages']);
    const {languages} = namespace;
    const {LanguageDescription} = owner;
    assert.deepEqual(languages.map(metadata), oracle.catalog.map(row => row.slice(0, 4)));
    for (const [file, expected] of oracle.filenames) {
        assert.equal(id(languages, LanguageDescription.matchFilename(languages, file)), expected, file);
    }
    for (const [name, fuzzy, expected] of oracle.names) {
        assert.equal(id(languages, LanguageDescription.matchLanguageName(languages, name, fuzzy)), expected, name);
    }
    const priority = [LanguageDescription.of({name: 'Extension', extensions: ['conf'], support: {}}),
        languages.find(d => d.name === 'Asterisk')];
    for (const [file, expected] of oracle.patternPriority) {
        assert.equal(id(priority, LanguageDescription.matchFilename(priority, file)), expected, file);
    }
    const records = await Promise.all(languages.map(async (d, i) => {
        assert.ok(d instanceof LanguageDescription, d.name);
        const byName = LanguageDescription.matchLanguageName(languages, d.name, false);
        const initial = d.support !== undefined;
        const pending = d.load();
        const waiter = byName.load();
        const supportWhilePending = byName.support !== undefined;
        const loaded = await pending;
        const legacy = oracle.catalog[i][4][0].startsWith('@codemirror/legacy-modes/');
        // CJS descriptors wrap legacy modes with CJS classes, but import all
        // modern targets as ESM. Compare each result with its actual owner.
        const target = legacy ? owner : esmOwner;
        assert.ok(loaded instanceof target.LanguageSupport, d.name);
        assert.equal(loaded.language instanceof owner.StreamLanguage, legacy, d.name);
        assert.equal(loaded.extension.length, 2, d.name);
        assert.equal(loaded.extension[0], loaded.language, d.name);
        assert.equal(loaded.extension[1], loaded.support, d.name);
        if (legacy) assert.deepEqual(loaded.support, [], d.name);
        assert.equal(loaded.language.parser.parse('x\n').length, 2, d.name);
        const {EditorState, Compartment} = legacy ? statePackage : esmState;
        const slot = new Compartment();
        const state = EditorState.create({doc: 'x\n', extensions: slot.of(loaded)});
        const tree = target.syntaxTree(state);
        const selected = state.update({selection: {anchor: 1}}).state;
        const edited = selected.update({changes: {from: 0, to: 1, insert: 'y'}}).state;
        const removed = edited.update({effects: slot.reconfigure([])}).state;
        const attachment = [
            state.facet(target.language) === loaded.language && tree.length === state.doc.length,
            target.syntaxTree(selected) === tree,
            target.syntaxTree(edited) !== tree && state.doc.toString() === 'x\n',
            removed.facet(target.language) === null && target.syntaxTree(removed).length === 0,
        ];
        assert.deepEqual(attachment, [true, true, true, true], d.name);
        return {
            pending, loaded, attachment,
            trace: [initial, byName === d, waiter === pending, supportWhilePending,
                await waiter === loaded, byName.support === loaded, byName.load() === pending],
        };
    }));
    const perDescription = records[0].trace;
    assert.deepEqual(perDescription, [false, true, true, false, true, true, true]);
    for (const {trace} of records) assert.deepEqual(trace, perDescription);
    const distinct = sharedRecipes.map(([left, right]) => [left, right,
        records[left].pending !== records[right].pending, records[left].loaded !== records[right].loaded]);
    for (const [left, right, attempts, supports] of distinct) {
        assert.ok(attempts && supports);
        const modern = !oracle.catalog[left][4][0].startsWith('@codemirror/legacy-modes/');
        assert.equal(records[left].loaded.language === records[right].loaded.language, modern);
    }
    const stockAttachments = [0, 1, 2, 3].map(i =>
        records.filter(row => row.attachment[i]).length);
    return {count: records.length, perDescription, distinct, stockAttachments};
}

const root = realpathSync(mkdtempSync(join(tmpdir(), 'language-data-catalog-')));
try {
    const modules = pinnedModules(resolve(process.argv[2]), root);
    const esmOwner = await modules.load(ownerPackage);
    const esmState = await modules.load('@codemirror/state');
    const {url} = modules.install(catalogPackage);
    const require = createRequire(url);
    const cjsOwner = require(ownerPackage);
    const cjsState = require('@codemirror/state');
    const esm = await modules.load(catalogPackage);
    const cjs = require(catalogPackage);
    assert.notEqual(esm.languages, cjs.languages);
    await assertRecipes(readFileSync(url, 'utf8'), esmOwner, false);
    await assertRecipes(readFileSync(require.resolve(catalogPackage), 'utf8'), cjsOwner, true);
    const traced = await capture(esm, esmOwner, esmOwner, esmState, esmState);
    assert.deepEqual(await capture(cjs, cjsOwner, esmOwner, cjsState, esmState), traced);
    console.log(JSON.stringify(traced));
} finally {
    rmSync(root, {recursive: true, force: true});
}
