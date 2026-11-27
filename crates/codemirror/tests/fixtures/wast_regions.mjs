// Run with Node and the directory of SHA-512-pinned npm archives.
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {projectedOffsets} from './editor_line_offsets.mjs';
import {pinnedModules} from './pinned_modules.mjs';

const cases = [
    ['root includes an empty document', 'root', ''],
    ['root includes all text', 'root', ';; 😀\r\n(module\r  (func)\n)'],
    ['file catalog loads WAT', 'catalog', '(module)', 'draft.wat'],
    ['file catalog loads WAST', 'catalog', '(module)', 'draft.wast'],
    ['side-by-side namespaces', 'controlled', '```wast\r(module)\r```\r\r```json\r{}\r```'],
    ['quoted overlay segments', 'controlled', '前 😀\r\n\r\n> ```wat extra\r> (module\r\n>   (func))\n> ```'],
    ['root regions exclude surrounding prose', 'controlled', 'Before\n\n```wast\n(module)\n```\n\nAfter'],
    ['unclosed fence includes the last line', 'controlled', '```wat\r\n(module\r  (func))'],
    ['unlabelled default and indented block', 'fallback', '```\r\n(module)\r\n```\r\n\r\n    (func)\r\n'],
    ['stock has no code resolver', 'stock', '```wat\r(module)\r```'],
    ['unknown or empty fence', 'controlled', '```unknown\n(module)\n```\n\n```wat\n```'],
    ['body consisting of a line break', 'controlled', '```wast\r\n\r\n```'],
];
assert.equal(process.argv.length, 3, 'usage: node wast_regions.mjs <pinned archive directory>');
const scratch = mkdtempSync(join(realpathSync(tmpdir()), 'gpui-wast-regions.'));
try {
    const {load} = pinnedModules(resolve(process.argv[2]), scratch);
    const {wast, wastLanguage} = await load('@codemirror/lang-wast');
    const {jsonLanguage} = await load('@codemirror/lang-json');
    const {markdown} = await load('@codemirror/lang-markdown');
    const {languages} = await load('@codemirror/language-data');
    const {LanguageDescription, ensureSyntaxTree} = await load('@codemirror/language');
    const {EditorState} = await load('@codemirror/state');
    const codeLanguages = name => ({wast: wastLanguage, wat: wastLanguage, json: jsonLanguage})[name];
    for (const [name, mode, source, filename] of cases) {
        let extension;
        switch (mode) {
        case 'root': extension = wastLanguage; break;
        case 'catalog': {
            const description = LanguageDescription.matchFilename(languages, filename);
            extension = await description.load();
            assert.equal(extension.language, wastLanguage);
            assert.equal(wast().language, extension.language);
            break;
        }
        case 'controlled': extension = markdown({codeLanguages}); break;
        case 'fallback': extension = markdown({codeLanguages, defaultCodeLanguage: wastLanguage}); break;
        case 'stock': extension = markdown(); break;
        default: assert.fail(mode);
        }
        const state = EditorState.create({doc: source, extensions: extension});
        assert(ensureSyntaxTree(state, state.doc.length, 5000));
        const {byteAt} = projectedOffsets(source);
        const bytes = Buffer.from(source);
        const regions = wastLanguage.findRegions(state).map(({from, to}) => {
            const range = [byteAt(from), byteAt(to)];
            return {range, raw: bytes.subarray(...range).toString()};
        });
        console.log(JSON.stringify({name, mode, source, filename, regions}));
    }
} finally {
    rmSync(scratch, {recursive: true, force: true});
}
