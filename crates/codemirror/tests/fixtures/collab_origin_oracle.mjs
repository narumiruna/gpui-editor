// node crates/codemirror/tests/fixtures/collab_origin_oracle.mjs <archive-root>
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {pinnedModules} from './pinned_modules.mjs';

const root = realpathSync(mkdtempSync(join(tmpdir(), 'collab-origin-')));
try {
    const {load, pins} = pinnedModules(resolve(process.argv[2]), root);
    assert.equal(pins.get('@codemirror/collab').version, '6.1.1');
    assert.equal(pins.get('@codemirror/state').version, '6.7.6');
    const {ChangeSet, EditorState, StateEffect} = await load('@codemirror/state');
    const {collab, getSyncedVersion, rebaseUpdates, receiveUpdates, sendableUpdates} = await load('@codemirror/collab');

    const seen = [];
    const protect = [2, 4];
    const mark = StateEffect.define({map: ([label, pos], changes) => [label, changes.mapPos(pos, 1)]});
    const origin = tr => ({
        start: [tr.startState.doc.toString(), tr.startState.selection.main.head],
        result: [tr.newDoc.toString(), tr.newSelection.main.head],
        changes: tr.changes.toJSON(),
        explicit: tr.selection?.main.head ?? null,
    });
    let state = EditorState.create({doc: 'abcd', selection: {anchor: 2}, extensions: [
        collab({clientID: 'a', sharedEffects(tr) {
            seen.push(tr);
            if (!tr.docChanged) return [];
            const {start, result} = origin(tr);
            return [mark.of([`${start.join(':')}>${result.join(':')}`, result[1]])];
        }}),
        EditorState.changeFilter.of(tr => tr.isUserEvent('delete') ? protect : true),
    ]});
    const queue = updates => updates.map(u => ({
        client: u.clientID, changes: u.changes.toJSON(), marks: u.effects.map(e => e.value),
        origin: u.origin ? origin(u.origin) : null,
    }));
    const snapshot = () => ({
        doc: state.doc.toString(), cursor: state.selection.main.head,
        version: getSyncedVersion(state), calls: seen.length, pending: queue(sendableUpdates(state)),
    });
    const start = snapshot();
    const requests = [
        {changes: ChangeSet.of({from: 1, insert: '!'}, 4).toJSON(), userEvent: 'input'},
        {changes: ChangeSet.of({from: 0, to: 5}, 5).toJSON(), userEvent: 'delete'},
    ];
    const local = requests.map(r => {
        state = state.update({...r, changes: ChangeSet.fromJSON(r.changes)}).state;
        return snapshot();
    });
    const peer = {clientID: 'b', changes: ChangeSet.of({from: 0, insert: '?'}, 4)};
    const pending = sendableUpdates(state);
    pending.forEach((u, i) => assert.equal(u.origin, seen[i]));
    const authority = queue(rebaseUpdates(pending, [peer]));
    assert.deepEqual(queue(rebaseUpdates(pending, [{...peer, changes: peer.changes.desc}])), authority);
    state = receiveUpdates(state, [peer]).state;
    const received = snapshot();
    const echoes = sendableUpdates(state);
    echoes.forEach((u, i) => assert.equal(u.origin, seen[i]));
    state = receiveUpdates(state, echoes.slice(0, 1)).state;
    const partial = snapshot();
    assert.equal(sendableUpdates(state)[0].origin, seen[1]);
    state = receiveUpdates(state, echoes.slice(1)).state;
    const acknowledged = snapshot();
    console.log(JSON.stringify({start, requests, protect,
        peer: {client: peer.clientID, changes: peer.changes.toJSON()},
        local, authority, received, partial, acknowledged}));
} finally {
    rmSync(root, {recursive: true, force: true});
}
