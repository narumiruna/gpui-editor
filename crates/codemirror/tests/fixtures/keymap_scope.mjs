// Run with Node and a directory of SHA-512-pinned npm archives.
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {pinnedModules} from './pinned_modules.mjs';

assert.equal(process.argv.length, 3, 'usage: node keymap_scope.mjs <archive directory>');
const scratch = mkdtempSync(join(realpathSync(tmpdir()), 'gpui-keymap-scope.'));
try {
    const {load} = pinnedModules(resolve(process.argv[2]), scratch);
    const {EditorState} = await load('@codemirror/state');
    const {keymap, runScopeHandlers} = await load('@codemirror/view');
    for (const bindingScope of ['', 'editor panel', 'editor  panel', 'editor editor', ' ', 'editor\tpanel', 'editor\u00a0panel', ' editor ']) {
        const calls = [];
        const view = {state: EditorState.create({extensions: keymap.of([
            {scope: bindingScope, key: 'ArrowLeft', any() { calls.push('decline'); return false; },
                run() { calls.push('move'); return true; }, stopPropagation: true},
        ])})};
        for (const eventScope of ['', 'editor', 'panel', 'editor\tpanel', 'editor\u00a0panel']) {
            for (const key of ['ArrowLeft', 'ArrowRight']) {
                calls.length = 0;
                let stopped = false;
                const handled = runScopeHandlers(view, {key, keyCode: key === 'ArrowLeft' ? 37 : 39,
                    stopPropagation() { stopped = true; }}, eventScope);
                console.log(JSON.stringify({bindingScope, eventScope, key, calls, handled, stopped}));
            }
        }
    }
} finally {
    rmSync(scratch, {recursive: true, force: true});
}
