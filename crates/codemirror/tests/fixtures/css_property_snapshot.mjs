// Replay with Node and the directory of SHA-512-pinned archives. Add --emit to regenerate.
import assert from 'node:assert/strict';
import {mkdtempSync, readFileSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {pinnedModules} from './pinned_modules.mjs';

// The host captures only enumerable keys with primitive string values. The
// completion source excludes cssText/cssFloat and normalizes the remaining keys.
const custom = Object.assign(Object.create({borderTop: '', inheritedName: '', hidden: ''}), {
    zIndex: '', backgroundColor: 'red', 'background-color': '', WebkitMask: '', webkitMask: '',
    '-webkit-mask': '', 'webkit-mask': '', MozBox: '', msGridRow: '', XMLThing: '', cssText: '',
    cssFloat: '', 'css-float': '', float: '', '2': 'color', '11': '', '013': '', length: 2,
    parentRule: null, item() {}, 'some-bool': true, 'boxed-string': new String('boxed'), ÉVariant: '',
    '\ue000': '', '𐀀': '', shorthand: undefined, [Symbol('not a string key')]: '',
});
Object.defineProperty(custom, 'hidden', {value: 'not enumerable'});
const empty = {cssText: '', cssFloat: '', length: 0, item() {}};

// The first nine cases select the property list; the rest are controls.
const cases = [
    ['css', '/*😀*/\r\na { col§or: red }'],
    ['css-helper', 'a { col§or: red }'],
    ['less', 'a { col§or: red }'],
    ['scss', 'a { col§or: red }'],
    ['sass', 'a\n  col§or: red'],
    ['css', 'a { § }', true],
    ['less', 'a { tag§ {} }'],
    ['html', '<div style="col§or: red"></div>'],
    ['html', '<style>a { col§or: red }</style>'],
    ['css', 'a { § }'],
    ['css', 'a { color: re§d }'],
    ['less', 'a:ho§ver {}'],
    ['sass', 'di§v\n  color: red'],
    ['scss', '@un§known;'],
    ['css', 'a { --one: red; color: var(--§) }'],
    ['less', 'a { @one: red; color: @on§e }'],
    ['scss', 'a { $one: red; color: $on§e }'],
];

function signature(options) {
    let hash = 2166136261;
    for (const {label, type, apply} of options)
        for (const byte of Buffer.from(`${label}\0${type}\0${apply ?? ''}\n`))
            hash = Math.imul(hash ^ byte, 16777619) >>> 0;
    return [options.length, hash];
}

async function capture(mode, style) {
    // Each extraction has fresh ESM identities and its own upstream property cache.
    const scratch = mkdtempSync(join(realpathSync(tmpdir()), 'gpui-css-property-snapshot.'));
    try {
        const {load} = pinnedModules(resolve(process.argv[2]), scratch);
        const {css, cssLanguage, defineCSSCompletionSource} = await load('@codemirror/lang-css');
        const {less} = await load('@codemirror/lang-less');
        const {sass} = await load('@codemirror/lang-sass');
        const {html} = await load('@codemirror/lang-html');
        const {EditorState} = await load('@codemirror/state');
        const {CompletionContext} = await load('@codemirror/autocomplete');
        const extensions = {
            css: css(), 'css-helper': [cssLanguage, cssLanguage.data.of({
                autocomplete: defineCSSCompletionSource(() => false),
            })],
            less: less(), scss: sass(), sass: sass({indented: true}), html: html(),
        };
        const keys = [];
        for (const key in style) if (typeof style[key] === 'string') keys.push(key);
        globalThis.document = {body: {style}};
        let shared;
        const rows = cases.map(([language, marked, explicit = false], index) => {
            const pos = marked.indexOf('§'), doc = marked.replace('§', '');
            const state = EditorState.create({doc, extensions: [extensions[language], EditorState.lineSeparator.of('\n')]});
            const sources = state.languageDataAt('autocomplete', pos);
            assert.equal(sources.length, 1);
            const result = sources[0](new CompletionContext(state, pos, explicit));
            const property = index < 9;
            if (property) {
                assert(result);
                assert.deepEqual(Object.keys(result), ['from', 'options', 'validFor']);
                assert.deepEqual(['', 'a', '-a', 'A0_-', '-', '--a', 'é', ' x', 'x:'].map(x => result.validFor.test(x)),
                    [true, true, true, true, false, false, false, false, false]);
                shared ??= result.options;
                assert.equal(result.options, shared);
                assert(result.options.every(({label, type, apply}) => type === 'property' && apply === `${label}: `));
            }
            const expected = result && [Buffer.byteLength(doc.slice(0, result.from)), ...signature(result.options)];
            return {language, marked, explicit, property, expected};
        });
        return {mode, keys, labels: shared.map(option => option.label), rows};
    } finally {
        delete globalThis.document;
        rmSync(scratch, {recursive: true, force: true});
    }
}

const actual = [await capture('custom', custom), await capture('empty', empty)].map(row => JSON.stringify(row)).join('\n') + '\n';
if (process.argv.includes('--emit')) process.stdout.write(actual);
else assert.equal(actual, readFileSync(new URL('./css_property_snapshot.jsonl', import.meta.url), 'utf8'));
