// Replay with Node and the directory of SHA-512-pinned archives.
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {projectedOffsets} from './editor_line_offsets.mjs';
import {pinnedModules} from './pinned_modules.mjs';

const scratch = mkdtempSync(join(realpathSync(tmpdir()), 'gpui-bracket-render.'));
try {
    const {load} = pinnedModules(resolve(process.argv[2]), scratch);
    const {bracketMatching} = await load('@codemirror/language');
    const {EditorSelection, EditorState} = await load('@codemirror/state');
    const {Decoration, ViewPlugin} = await load('@codemirror/view');
    const source = '(甲) [😀]';
    const {byteAt} = projectedOffsets(source);
    const before = [1, 5];
    const match = 'cm-matchingBracket';
    const miss = 'cm-nonmatchingBracket';
    const cases = {
        startOnly: hit => [Decoration.mark({class: miss}).range(hit.start.from, hit.start.to)],
        interior: hit => [Decoration.mark({class: match}).range(hit.start.to, hit.end.from)],
        empty: () => [],
        unstyled: hit => [Decoration.mark({class: 'host-custom'}).range(hit.start.from, hit.start.to)],
    };
    for (const [name, renderMatch] of Object.entries(cases)) {
        const matching = bracketMatching({renderMatch(hit, state) {
            assert.equal(state.doc.toString(), source);
            return renderMatch(hit);
        }});
        const plugin = matching.flat(Infinity).find(extension => extension instanceof ViewPlugin);
        assert(plugin);
        const state = EditorState.create({doc: source, selection: EditorSelection.create(before.map(at => EditorSelection.cursor(at))),
            extensions: [EditorState.allowMultipleSelections.of(true), matching]});
        const view = plugin.create({state});
        const marks = [];
        for (let mark = view.decorations.iter(); mark.value; mark.next()) {
            marks.push([byteAt(mark.from), byteAt(mark.to), mark.value.spec.class]);
        }
        console.log(JSON.stringify({name, source, carets: before.map(byteAt), marks}));
    }
} finally {
    rmSync(scratch, {recursive: true, force: true});
}
