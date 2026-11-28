// node crates/codemirror/tests/fixtures/collab_origin_queue.mjs <archive-root>
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {pinnedModules} from './pinned_modules.mjs';

const root = mkdtempSync(join(realpathSync(tmpdir()), 'collab-origin-queue.'));
try {
    const {load, pins} = pinnedModules(resolve(process.argv[2]), root);
    assert.equal(pins.get('@codemirror/collab').version, '6.1.1');
    assert.equal(pins.get('@codemirror/state').version, '6.7.6');
    const {ChangeSet, Compartment, EditorState, StateEffect, Transaction} = await load('@codemirror/state');
    const {collab, getClientID, getSyncedVersion, receiveUpdates, sendableUpdates} = await load('@codemirror/collab');
    const mark = StateEffect.define({map: (value, changes) => changes.mapPos(value, 1)});
    const slot = new Compartment();
    const time = tr => tr.annotation(Transaction.time);
    const queue = state => sendableUpdates(state).map(update => [update.clientID, update.changes.toJSON(),
        update.effects.map(effect => effect.value), time(update.origin)]);
    const snapshot = state => {
        const client = getClientID(state), doc = state.doc.toString();
        if (client === '') {
            assert.throws(() => getSyncedVersion(state), /Field is not present/);
            assert.throws(() => sendableUpdates(state), /Field is not present/);
            return {doc, client, version: null, queue: null};
        }
        return {doc, client, version: getSyncedVersion(state), queue: queue(state)};
    };
    const during = new Map();
    const config = (clientID, startVersion) => collab({clientID, startVersion, sharedEffects(tr) {
        const effects = tr.effects.filter(effect => effect.is(mark));
        if (effects.length) {
            const start = snapshot(tr.startState);
            const client = getClientID(tr.state);
            assert.throws(() => getSyncedVersion(tr.state), /Cyclic dependency/);
            assert.throws(() => sendableUpdates(tr.state), /Cyclic dependency/);
            during.set(time(tr), {start, resultClient: client});
        }
        return effects;
    }});
    const initial = {doc: 'abc', client: 'a', version: 7};
    let state = EditorState.create({doc: initial.doc, extensions: slot.of(config(initial.client, initial.version))});
    console.log(JSON.stringify({initial, live: snapshot(state)}));
    const actions = [
        {time: 11, changes: [1, [0, 'X'], 2], mark: 2},
        {time: 12, mark: 3},
        {peer: 'b', changes: [[0, 'yz'], 3]},
        {echo: 1},
        {time: 13, changes: [6, [0, '!']], mark: 7},
        {echo: 2},
        {time: 14, changes: [7, [0, '?']], mark: 8, config: ['renamed', 99]},
        {echo: 1},
        {remove: true},
        {time: 15, changes: [8, [0, '*']], config: ['fresh', 20]},
    ];
    const retained = new Map();
    for (const action of actions) {
        const {peer, echo, changes, mark: position, time, config: setting, remove} = action;
        if (peer || echo) {
            const updates = peer ? [{clientID: peer, changes: ChangeSet.fromJSON(changes)}]
                : sendableUpdates(state).slice(0, echo);
            state = receiveUpdates(state, updates).state;
        } else {
            const effects = [];
            if (position != null) effects.push(mark.of(position));
            if (setting || remove) effects.push(slot.reconfigure(setting ? config(...setting) : []));
            state = state.update({changes: changes && ChangeSet.fromJSON(changes), effects,
                annotations: time == null ? [] : Transaction.time.of(time)}).state;
        }
        if (!remove) {
            for (const update of sendableUpdates(state)) {
                const key = update.origin.annotation(Transaction.time);
                if (retained.has(key)) assert.equal(update.origin, retained.get(key).origin);
                retained.set(key, update);
            }
        }
        console.log(JSON.stringify({action, live: snapshot(state)}));
    }
    state = null;
    for (const [key, {origin}] of retained) {
        const start = snapshot(origin.startState), result = snapshot(origin.state);
        const earlier = start.queue == null ? [] : sendableUpdates(origin.startState);
        const current = sendableUpdates(origin.state);
        assert.equal(current.length, earlier.length + 1);
        earlier.forEach((update, index) => assert.equal(update, current[index]));
        assert.equal(current.at(-1).origin, origin);
        if (during.has(key)) assert.deepEqual(during.get(key), {start, resultClient: result.client});
        for (const update of [...earlier, ...current]) {
            assert.equal(update.origin, retained.get(time(update.origin)).origin);
        }
        console.log(JSON.stringify({time: key, start, result, callback: during.get(key) ?? null}));
    }
} finally {
    rmSync(root, {recursive: true, force: true});
}
