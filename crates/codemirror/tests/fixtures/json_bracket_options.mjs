// Run with Node and the directory of SHA-512-pinned archives.
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {projectedOffsets} from './editor_line_offsets.mjs';
import {pinnedModules} from './pinned_modules.mjs';

const forward = '["(😀",\r\n\r")"]';
const backward = '["(",\r\n\r"😀)"]';
const alternate = '["(«x»",\r\n")"]';
const cases = [
    ['custom pair replaces default pairs in strings', '["(x)","«x»"]', {brackets: '«»', maxScanDistance: 1}],
    ['custom mate in a string', '["(x]"]', {brackets: '(]'}],
    ['property names and strings have different syntax types', '{"«":"»"}', {brackets: '«»', maxScanDistance: 1}],
    ['grammar pair ignores distance and absent alternatives', '[\r\n0\r]', {brackets: '«»', maxScanDistance: 1}],
    ['grammar pair overrides a conflicting mate', '[0]', {brackets: '[)'}],
    ['a reversed grammar pair can also match by name', '[0]', {brackets: ']['}],
    ['named punctuation participates in marked matching', '{"a":0,\r\n"b":1}', {brackets: ',:'}],
    ['try the other adjacent bracket after cutoff', alternate, {brackets: '()«»', maxScanDistance: 1}],
    ['before-only mode has no alternate after cutoff', alternate, {brackets: '()«»', maxScanDistance: 1, afterCursor: false}],
    ['an adjacent mismatch stops the search', '["(«x»]"]', {brackets: '()[]«»', maxScanDistance: 1}],
    ['an adjacent unmatched result stops the search', '[")«x»"]', {brackets: '()«»'}],
    ['forward scan accepts the exact logical UTF-16 distance', forward, {maxScanDistance: 6}],
    ['forward scan cuts off before the next line', forward, {maxScanDistance: 5}],
    ['backward scan accepts the exact logical UTF-16 distance', backward, {maxScanDistance: 6}],
    ['backward scan cuts off before the previous line', backward, {maxScanDistance: 5}],
    ['same-line match may exceed the limit', '["(abcdefghijklmn)"]', {maxScanDistance: 1}],
    ['same-line miss returns unmatched at EOF', '["(abcdefghijklmn"]', {maxScanDistance: 1}],
    ['empty options use stock pairs and distance', forward, {brackets: '', maxScanDistance: 0}],
];
const scratch = mkdtempSync(join(realpathSync(tmpdir()), 'gpui-json-brackets.'));
try {
    const {load} = pinnedModules(resolve(process.argv[2]), scratch);
    const {json} = await load('@codemirror/lang-json');
    const {bracketMatching, forceParsing} = await load('@codemirror/language');
    const {EditorState} = await load('@codemirror/state');
    const {ViewPlugin} = await load('@codemirror/view');
    for (const [name, source, options] of cases) {
        const {byteAt} = projectedOffsets(source);
        const matching = bracketMatching(options);
        const plugin = matching.flat(Infinity).find(extension => extension instanceof ViewPlugin);
        assert(plugin);
        const view = {
            state: EditorState.create({doc: source, extensions: [json(), matching]}),
            dispatch(spec) { this.state = this.state.update(spec).state; },
        };
        assert(forceParsing(view, view.state.doc.length, 5000));
        const checks = [];
        let anchor = 0;
        for (const part of ['', ...source.match(/\r\n|[^]/gu)]) {
            anchor += part === '\r\n' ? 1 : part.length;
            const state = view.state.update({selection: {anchor}}).state;
            const marks = [];
            for (const iter = plugin.create({state}).decorations.iter(); iter.value; iter.next()) {
                const label = iter.value.spec.class;
                assert(label === 'cm-matchingBracket' || label === 'cm-nonmatchingBracket');
                marks.push([byteAt(iter.from), byteAt(iter.to), label === 'cm-matchingBracket']);
            }
            checks.push([byteAt(anchor), marks]);
        }
        console.log(JSON.stringify({name, source, options, checks}));
    }
} finally {
    rmSync(scratch, {recursive: true, force: true});
}
