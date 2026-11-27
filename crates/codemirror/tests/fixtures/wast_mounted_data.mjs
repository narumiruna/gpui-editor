// Run with Node and the directory of SHA-512-pinned npm archives.
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {projectedOffsets} from './editor_line_offsets.mjs';
import {pinnedModules} from './pinned_modules.mjs';

assert.equal(process.argv.length, 3, 'usage: node wast_mounted_data.mjs <pinned archive directory>');
const scratch = mkdtempSync(join(realpathSync(tmpdir()), 'gpui-wast-mounted-data.'));
try {
    const {load} = pinnedModules(resolve(process.argv[2]), scratch);
    const {Compartment, EditorState, Prec} = await load('@codemirror/state');
    const {ensureSyntaxTree} = await load('@codemirror/language');
    const {markdown} = await load('@codemirror/lang-markdown');
    const {wastLanguage} = await load('@codemirror/lang-wast');
    const renamed = wastLanguage.configure({}, 'named-wast');
    assert.equal(wastLanguage.data, renamed.data);
    const selected = markdown({codeLanguages: name => ({wat: renamed, wast: wastLanguage})[name]});
    const stock = markdown();
    const high = Prec.high(renamed.data.of({wordChars: '@', closeBrackets: {brackets: ['[']}}));
    const low = wastLanguage.data.of({wordChars: '$'});
    function capture(step, state, source) {
        const {units, byteAt} = projectedOffsets(source);
        assert(ensureSyntaxTree(state, units, 5000));
        const positions = [0];
        for (const part of source.match(/\r\n|[^]/gu) ?? []) {
            positions.push(positions.at(-1) + (part === '\r\n' ? 1 : part.length));
        }
        const regions = wastLanguage.findRegions(state);
        const at = new Set([0, units]);
        for (const {from, to} of regions) {
            for (const point of [from, to]) {
                const index = positions.indexOf(point);
                assert(index >= 0);
                for (const pos of positions.slice(Math.max(0, index - 1), index + 2)) at.add(pos);
            }
        }
        const data = (pos, side) => ({
            active: wastLanguage.isActiveAt(state, pos, side),
            words: state.languageDataAt('wordChars', pos, side),
            comments: state.languageDataAt('commentTokens', pos, side),
            brackets: state.languageDataAt('closeBrackets', pos, side).map(data => data.brackets),
            completionSources: state.languageDataAt('autocomplete', pos, side).length,
        });
        const probes = [...at].sort((a, b) => a - b).map(pos => [byteAt(pos), data(pos, -1), data(pos, 1)]);
        console.log(JSON.stringify({step, source, regions: regions.map(({from, to}) => [byteAt(from), byteAt(to)]), probes}));
    }
    const original = '前 😀\r\n\r\n> ```wat\r\n> (module\r>   (func))\r> ```\r\n\r\n```wast\r(func)\r';
    const parser = new Compartment(), extras = new Compartment();
    let state = EditorState.create({doc: original, extensions: [parser.of(selected), extras.of([high, low])]});
    const retained = state;
    capture('configured mounts', state, original);
    state = state.update({effects: extras.reconfigure([])}).state;
    capture('remove shared metadata', state, original);
    const insert = 'Lead\n\n';
    state = state.update({changes: {from: 0, insert}}).state;
    capture('shift the mounts', state, insert + original);
    state = state.update({effects: parser.reconfigure(stock)}).state;
    capture('remove WAST mounts', state, insert + original);
    capture('retained', retained, original);
} finally {
    rmSync(scratch, {recursive: true, force: true});
}
