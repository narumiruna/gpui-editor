// Run with Node and a directory of local SHA-512-pinned npm archives.
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {pinnedModules} from './pinned_modules.mjs';

assert.equal(process.argv.length, 3, 'provide the directory of local npm archives');
const scratch = mkdtempSync(join(realpathSync(tmpdir()), 'collab-state-context.'));
try {
    const {load} = pinnedModules(resolve(process.argv[2]), scratch);
    const {ChangeSet, Compartment, EditorState, Facet, StateEffect, StateField, Transaction} = await load('@codemirror/state');
    const {collab, getSyncedVersion, receiveUpdates, sendableUpdates} = await load('@codemirror/collab');
    const effect = StateEffect.define();
    const share = StateEffect.define();
    const tally = StateField.define({create: () => 0, update: (value, tr) => value + tr.effects.filter(e => e.is(effect)).length});
    const label = Facet.define({combine: values => values[0] ?? ''});
    const total = Facet.define({combine: values => values[0] ?? 0});
    const slot = new Compartment();
    const snapshot = state => [state.field(tally), state.facet(label), state.facet(total), state.doc.toString(),
        Buffer.byteLength(state.doc.sliceString(0, state.selection.main.head))];
    const inspect = tr => [snapshot(tr.startState), snapshot(tr.state), tr.annotation(Transaction.time)];
    const config = collab({clientID: 'a', startVersion: 7, sharedEffects(tr) {
        return tr.effects.some(e => e.is(effect)) ? [share.of(inspect(tr))] : [];
    }});
    const initial = 'ab😀';
    let state = EditorState.create({doc: initial, extensions: [config, tally,
        total.compute([tally], state => state.field(tally) * 2), slot.of(label.of('before'))]});
    const at = 1;
    state = state.update({effects: [effect.of(null), slot.reconfigure(label.of('after'))], selection: {anchor: at}, annotations: Transaction.time.of(19)}).state;
    const queue = () => sendableUpdates(state).map(update => {
        assert(update.changes.empty);
        assert.deepEqual(update.effects.map(effect => effect.value), [inspect(update.origin)]);
        return update.effects[0].value;
    });
    const record = () => ({version: getSyncedVersion(state), state: snapshot(state), queue: queue()});
    const before = record();
    const peer = '字';
    state = receiveUpdates(state, [{clientID: 'b', changes: ChangeSet.of({from: 0, insert: peer}, state.doc.length)}]).state;
    const received = record();
    state = receiveUpdates(state, sendableUpdates(state)).state;
    console.log(JSON.stringify({initial, at, peer, before, received, acknowledged: record()}));
} finally {
    rmSync(scratch, {recursive: true, force: true});
}
