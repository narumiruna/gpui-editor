// Run with Node and the directory of SHA-512-pinned npm archives.
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {projectedOffsets} from './editor_line_offsets.mjs';
import {pinnedModules} from './pinned_modules.mjs';

const cases = [
    ['no attachment', 'off', 'off'],
    ['JSON alone', 'json', 'off'],
    ['JSON before WAST', 'json', 'wast'],
    ['WAST before JSON', 'wast', 'json'],
    ['high precedence WAST', 'json', 'high-wast'],
    ['high precedence JSON', 'wast', 'high-json'],
    ['same JSON language installed twice', 'json', 'json'],
    ['same WAST language installed twice', 'wast', 'wast'],
    ['detach the first language', 'off', 'wast'],
    ['reattach JSON first', 'json', 'wast'],
    ['detach both', 'off', 'off'],
];
assert.equal(process.argv.length, 3, 'usage: node attached_language_precedence.mjs <pinned archive directory>');
const scratch = mkdtempSync(join(realpathSync(tmpdir()), 'gpui-attached-language-precedence.'));
try {
    const {load} = pinnedModules(resolve(process.argv[2]), scratch);
    const {Compartment, EditorState, Prec} = await load('@codemirror/state');
    const {foldable, getIndentation, syntaxTree} = await load('@codemirror/language');
    const {json, jsonLanguage} = await load('@codemirror/lang-json');
    const {wast, wastLanguage} = await load('@codemirror/lang-wast');
    const extension = name => {
        if (name === 'off') return [];
        const language = name.endsWith('json') ? json() : wast();
        return name.startsWith('high-') ? Prec.high(language) : language;
    };
    const source = '{\r\n  "😀": 1\r}';
    const byte = projectedOffsets(source).byteAt;
    const compartments = [new Compartment(), new Compartment()];
    const host = EditorState.languageData.of(() => [{wordChars: '@'}]);
    let state = EditorState.create({doc: source, extensions: [host, ...compartments.map(slot => slot.of([]))]});
    const latest = new Map();
    for (const [name, ...attachments] of cases) {
        state = state.update({effects: compartments.map((slot, at) => slot.reconfigure(extension(attachments[at])))}).state;
        const fold = foldable(state, state.doc.line(1).from, state.doc.line(1).to);
        const before = state.doc.line(2).from;
        const language = jsonLanguage.isActiveAt(state, before) ? 'json' : wastLanguage.isActiveAt(state, before) ? 'wast' : null;
        const tree = syntaxTree(state);
        const reused = latest.has(language) ? latest.get(language) === tree : null;
        latest.set(language, tree);
        const metadata = position => ({closeBrackets: state.languageDataAt('closeBrackets', position).map(data => data.brackets),
            commentTokens: state.languageDataAt('commentTokens', position), wordChars: state.languageDataAt('wordChars', position)});
        const positions = [0, before, state.doc.length];
        console.log(JSON.stringify({name, source, attachments, language, tree: tree.topNode.name, reused,
            indent: getIndentation(state, before) ?? null,
            fold: fold ? [byte(fold.from), byte(fold.to)] : null,
            lineEnd: byte(state.doc.line(1).to), indentAt: byte(before),
            metadata: positions.map(pos => [byte(pos), metadata(pos)])}));
    }
} finally {
    rmSync(scratch, {recursive: true, force: true});
}
