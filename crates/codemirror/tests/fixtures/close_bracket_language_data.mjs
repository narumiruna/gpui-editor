// Run with Node and the directory of SHA-512-pinned npm archives.
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {projectedOffsets, projectInput} from './editor_line_offsets.mjs';
import {pinnedModules} from './pinned_modules.mjs';

const cases = [
    {name: 'configured before allows a comma', source: ',', heads: [0], input: '(', before: ','},
    {name: 'configured before replaces defaults', source: ']', heads: [0], input: '(', before: ','},
    {name: 'an empty before uses the default', source: ']', heads: [0], input: '(', before: ''},
    {name: 'whitespace does not need to be listed', source: '\uFEFF', heads: [0], input: '(', before: ','},
    {name: 'word at the previous unit', source: '$ ', heads: [1], input: '"', extra: [[1, '$']]},
    {name: 'word at the next character', source: ' $', heads: [1], input: '"', extra: [[1, '$']]},
    {name: 'prefix uses the caret word rule', source: '$r ', heads: [2], input: '"', prefixes: ['r'], extra: [[2, '$']]},
    {name: 'a word rule at the prefix start is not used', source: '$r ', heads: [2], input: '"', prefixes: ['r'], extra: [[1, '$']]},
    {name: 'space is not a word', source: '  ', heads: [1], input: '"', extra: [[1, ' ']]},
    {name: 'second caret can veto an automatic pair', source: '\r\n$ ', heads: [0, 2], input: '"', extra: [[2, '$']]},
    {name: 'word rule does not follow the main caret', source: '\r\n$ ', heads: [0, 2], main: 1, input: '"', extra: [[0, '$']]},
    {name: 'low surrogate identifies the previous unit', source: '😀 ', heads: [2], input: '"', extra: [[2, '🨀']]},
    {name: 'a distinct low surrogate does not block', source: '😀 ', heads: [2], input: '"', extra: [[2, '😁']]},
    {name: 'high surrogate can match part of the next character', source: ' 😀', heads: [1], input: '"', extra: [[1, '😁']]},
];
assert.equal(process.argv.length, 3, 'usage: node close_bracket_language_data.mjs <pinned archive directory>');
const scratch = mkdtempSync(join(realpathSync(tmpdir()), 'gpui-close-bracket-data.'));
try {
    const {load} = pinnedModules(resolve(process.argv[2]), scratch);
    const {closeBrackets, insertBracket} = await load('@codemirror/autocomplete');
    const {EditorSelection, EditorState} = await load('@codemirror/state');
    for (const {name, source, heads, main = 0, input, before, prefixes, extra = []} of cases) {
        const close = {before, stringPrefixes: prefixes};
        const rules = new Map(extra);
        const state = EditorState.create({doc: source,
            selection: EditorSelection.create(heads.map(head => EditorSelection.cursor(head)), main),
            extensions: [closeBrackets(), EditorState.allowMultipleSelections.of(true),
                EditorState.languageData.of((_, pos) => [rules.has(pos)
                    ? {closeBrackets: close, wordChars: rules.get(pos)} : {closeBrackets: close}])]});
        const tr = insertBracket(state, input);
        const {byteAt} = projectedOffsets(source);
        const result = tr ? projectInput(source, tr) : null;
        const offsets = result === null ? null : projectedOffsets(result);
        console.log(JSON.stringify({kind: 'input', name, source, heads: heads.map(byteAt), main, input, before, prefixes,
            words: extra.map(([pos, chars]) => [byteAt(pos), chars]), result,
            selection: tr ? tr.newSelection.ranges.map(range => offsets.byteAt(range.head)) : null}));
    }
    for (const [name, separator, lineBreak, keep] of [
        ['lone CR separates logical lines', undefined, '\r', false],
        ['CRLF separates logical lines', undefined, '\r\n', false],
        ['explicit LF keeps a lone CR within one line', '\n', '\r', true],
        ['no separator between the carets', undefined, ' ', true],
    ]) {
        const source = ` 😀${lineBreak}j`;
        const extensions = [closeBrackets()];
        if (separator !== undefined) extensions.push(EditorState.lineSeparator.of(separator));
        let state = EditorState.create({doc: source, extensions});
        state = insertBracket(state, '(').state;
        state = state.update({selection: {anchor: state.doc.length}}).state;
        state = state.update({selection: {anchor: 1}}).state;
        const skip = insertBracket(state, ')');
        assert.equal(Boolean(skip), keep, name);
        if (skip) assert.equal(skip.newSelection.main.head, 2);
        console.log(JSON.stringify({kind: 'track', name, source, separator, keep}));
    }
} finally {
    rmSync(scratch, {recursive: true, force: true});
}
