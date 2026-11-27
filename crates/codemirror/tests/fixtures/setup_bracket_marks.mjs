// Run with Node and the directory of SHA-512-pinned npm archives.
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {projectedOffsets} from './editor_line_offsets.mjs';
import {pinnedModules} from './pinned_modules.mjs';

const cases = [
    ['JSON structure skips string punctuation', 'json', '{|\r\n  "😀":"}"\r}', {}],
    ['WAST structure skips comment punctuation', 'wast', '(|module ;; 😀 )\r\n)', {}],
    ['WAST comments and structure produce separate marks', 'wast', ';; «|😀»\r\n(|module)', {brackets: '«»', maxScanDistance: 1}],
    ['WAST mixed carets at matching and unmatched punctuation', 'wast', ';; («|q»\r\n(module)| (; |', {brackets: '()«»', maxScanDistance: 1}],
    ['JSON cutoff permits the other caret probe', 'json', '["(|«x»",\r\n")"]', {brackets: '()«»', maxScanDistance: 1}],
    ['JSON before-only forbids the other probe', 'json', '["(|«x»",\r\n")"]', {brackets: '()«»', maxScanDistance: 1, afterCursor: false}],
    ['WAST renderer receives tree-based matches', 'wast', '(|module ;; 😀 )\r\n)\r;; )|', {render: true}],
    ['JSON renderer can choose a single span', 'json', '{|"a":"}"}\r\n[|0]', {render: true}],
];
assert.equal(process.argv.length, 3, 'usage: node setup_bracket_marks.mjs <pinned archive directory>');
const scratch = mkdtempSync(join(realpathSync(tmpdir()), 'gpui-setup-bracket-marks.'));
try {
    const {load} = pinnedModules(resolve(process.argv[2]), scratch);
    const {basicSetup} = await load('codemirror');
    const {bracketMatching} = await load('@codemirror/language');
    const {Compartment, EditorState, EditorSelection} = await load('@codemirror/state');
    const {Decoration, ViewPlugin} = await load('@codemirror/view');
    const {json} = await load('@codemirror/lang-json');
    const {wast} = await load('@codemirror/lang-wast');
    for (const [name, mode, annotated, options] of cases) {
        const parts = annotated.split('|');
        const source = parts.join('');
        const {byteAt} = projectedOffsets(source);
        let offset = 0;
        const carets = parts.slice(0, -1).map(part => offset += part.replace(/\r\n?/g, '\n').length);
        const selection = EditorSelection.create(carets.map(pos => EditorSelection.cursor(pos)));
        const callbacks = [];
        const range = span => span ? [byteAt(span.from), byteAt(span.to)] : null;
        const {render, ...config} = options;
        const extension = bracketMatching(render ? {...config, renderMatch(hit, state) {
            assert.equal(state.doc.toString(), source.replace(/\r\n?/g, '\n'));
            callbacks.push([range(hit.start), range(hit.end), hit.matched, state.selection.ranges.length]);
            const className = hit.matched ? 'syntax-match' : 'syntax-miss';
            return [Decoration.mark({class: className}).range(hit.start.from, hit.start.to)];
        }} : config);
        const plugin = extension.flat(Infinity).find(item => item instanceof ViewPlugin);
        assert(plugin && basicSetup.flat(Infinity).includes(plugin));
        const slot = new Compartment();
        let state = EditorState.create({doc: source, selection,
            extensions: [mode === 'json' ? json() : wast(), basicSetup, slot.of(extension)]});
        const marks = () => {
            const result = [];
            for (const it = plugin.create({state}).decorations.iter(); it.value; it.next()) {
                result.push([...range(it), it.value.spec.class]);
            }
            return result;
        };
        const before = marks();
        const observed = callbacks.slice();
        state = state.update({effects: slot.reconfigure([])}).state;
        const after = marks();
        assert.deepEqual(callbacks, observed);
        console.log(JSON.stringify({name, mode, source, options, carets: carets.map(byteAt), before, callbacks: observed, after}));
    }
} finally {
    rmSync(scratch, {recursive: true, force: true});
}
