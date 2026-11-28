// Run with Node and a directory of SHA-512-pinned npm archives.
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {pinnedModules} from './pinned_modules.mjs';
import {projectedOffsets} from './editor_line_offsets.mjs';

assert.equal(process.argv.length, 3, 'usage: node autocomplete_list_prefix.mjs <archive directory>');
const scratch = mkdtempSync(join(realpathSync(tmpdir()), 'gpui-completion-list.'));
try {
    const {load} = pinnedModules(resolve(process.argv[2]), scratch);
    const {EditorState} = await load('@codemirror/state');
    const {CompletionContext, completeFromList, insertCompletionText} = await load('@codemirror/autocomplete');
    const empty = {label: '', displayLabel: 'Insert default', apply: 'default'};
    const note = {label: '!note', displayLabel: 'Note action', apply: '!note'};
    for (const [name, left, right, options, pick] of [
        ['mixed list', 'é no', ' +', [empty, note], '!note'],
        ['empty label only', 'é no', ' +', [empty]],
        ['symbolic label only', 'é no', ' +', [note]],
        ['already symbolic', 'é !no', ' +', [empty, note]],
        ['word list after CRLF', 'prior\r\né al', '=arg', ['alpha', 'allow']],
        ['empty option list', 'é no', ' +', []],
    ]) for (const explicit of [false, true]) {
        const source = left + right;
        const pos = projectedOffsets(left).units;
        const {byteAt} = projectedOffsets(source);
        const state = EditorState.create({doc: source, selection: {anchor: pos}});
        const found = completeFromList(options)(new CompletionContext(state, pos, explicit));
        const result = found && {from: byteAt(found.from), to: found.to == null ? null : byteAt(found.to), options: found.options};
        const picked = pick && found && found.options.find(option => option.label === pick);
        const edit = picked ? state.update(insertCompletionText(state, picked.apply ?? picked.label, found.from, found.to ?? pos)).newDoc.toString() : undefined;
        console.log(JSON.stringify({name, source, at: byteAt(pos), options, explicit, result, pick, edit}));
    }
} finally {
    rmSync(scratch, {recursive: true, force: true});
}
