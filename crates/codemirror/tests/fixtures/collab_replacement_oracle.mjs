// node crates/codemirror/tests/fixtures/collab_replacement_oracle.mjs <archive-root>
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {pinnedModules} from './pinned_modules.mjs';

const root = realpathSync(mkdtempSync(join(tmpdir(), 'collab-same-text-')));
try {
    const {load} = pinnedModules(resolve(process.argv[2]), root);
    const {ChangeSet, EditorState, StateEffect} = await load('@codemirror/state');
    const {collab, getClientID, getSyncedVersion, receiveUpdates, sendableUpdates} = await load('@codemirror/collab');
    const source = 'aé😀z';
    const state = EditorState.create({doc: source, selection: {anchor: 4}, extensions: collab({clientID: 'a'})});
    const changes = ChangeSet.of({from: 0, to: source.length, insert: source}, source.length);
    const snapshot = state => ({text: state.doc.toString(), cursor: Buffer.byteLength(state.doc.sliceString(0, state.selection.main.head)),
        pending: sendableUpdates(state).length, version: getSyncedVersion(state)});
    const local = state.update({changes}).state;
    const echo = receiveUpdates(local, sendableUpdates(local)).state;
    const remote = receiveUpdates(state, [{clientID: 'b', changes}]).state;
    const start = {text: 'ab', cursor: 1};
    const config = {clientID: 'joined', startVersion: 7};
    const joined = EditorState.create({doc: start.text, selection: {anchor: start.cursor}}).update({
        changes: {from: 0, to: start.text.length, insert: start.text},
        effects: StateEffect.appendConfig.of(collab(config)),
    }).state;
    const installation = {start, config, installed: {
        ...snapshot(joined), client: getClientID(joined),
        queue: sendableUpdates(joined).map(({clientID, changes}) => ({client: clientID, changes: changes.toJSON()})),
    }};
    console.log(JSON.stringify({source, start: snapshot(state), local: snapshot(local), echo: snapshot(echo), remote: snapshot(remote), installation}));
} finally {
    rmSync(root, {recursive: true, force: true});
}
