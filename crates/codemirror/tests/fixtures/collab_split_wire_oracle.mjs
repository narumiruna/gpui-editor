// node crates/codemirror/tests/fixtures/collab_split_wire_oracle.mjs <archive-root> \
//   > crates/codemirror/tests/fixtures/collab_split_wire_oracle.jsonl
import assert from 'node:assert/strict';
import {existsSync, mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {pinnedModules} from './pinned_modules.mjs';

const archives = resolve(process.argv[2]);
const root = realpathSync(mkdtempSync(join(tmpdir(), 'collab-split-wire-')));
try {
    const {load, pins} = pinnedModules(archives, root);
    assert.equal(pins.get('@codemirror/collab').version, '6.1.1');
    assert.equal(pins.get('@codemirror/state').version, '6.7.6');
    // Require local archives so the loader does not fetch missing packages.
    for (const name of ['@codemirror/collab', '@codemirror/state', '@marijn/find-cluster-break']) {
        const file = `${name.split('/').at(-1)}-${pins.get(name).version}.tgz`;
        const path = join(archives, name.replace('/', '__'), file);
        assert(existsSync(path), `missing local archive: ${path}`);
    }
    const {ChangeSet, EditorState, MapMode, StateEffect, Transaction} = await load('@codemirror/state');
    const {collab, getSyncedVersion, rebaseUpdates, receiveUpdates, sendableUpdates} = await load('@codemirror/collab');
    const version = 7;
    const start = doc => EditorState.create({doc, extensions: collab({
        clientID: 'a', startVersion: version, sharedEffects: tr => tr.effects,
    })});
    const values = effects => effects.map(effect => effect.value);
    const wire = ({clientID, changes, effects = []}) => ({client: clientID, changes: changes.toJSON(), marks: values(effects)});
    const decode = ({client, changes, marks}, type) => ({
        clientID: client, changes: ChangeSet.fromJSON(changes), effects: marks.map(mark => type.of(mark)),
    });
    const applyLocal = (state, local, type) => local.reduce((state, edit) => {
        const {changes, effects} = decode(edit, type);
        return state.update({changes, effects}).state;
    }, state);
    const snapshot = state => ({
        doc: state.doc.toString(), version: getSyncedVersion(state), queue: sendableUpdates(state).map(wire),
    });

    // All text is ASCII, so the UTF-16 wire positions also give UTF-8 byte positions.
    {
        const source = 'a';
        const type = StateEffect.define();
        const local = [{changes: [1], marks: [['local', 0]]}, {changes: [[0, '!'], 1], marks: []}];
        const prefix = [
            {client: 'b', changes: [[0, 'X'], [0, 'B'], 1], marks: []},
            {client: 'a', changes: [3], marks: []},
            {client: 'd', changes: [1, [0, 'Z'], [2]], marks: []},
        ];
        const base = start(source);
        const state = applyLocal(base, local, type);
        const over = prefix.map(edit => decode(edit, type));
        const authority = over.reduce((doc, update) => update.changes.apply(doc), base.doc);
        const pending = sendableUpdates(state);
        const concrete = rebaseUpdates(pending, over);
        const described = rebaseUpdates(pending, over.map(u => ({...u, changes: u.changes.desc})));
        const received = receiveUpdates(state, over).state;
        function replay(updates) {
            assert.equal(updates.length, 1);
            return {
                echo: wire(updates[0]), authority: updates[0].changes.apply(authority).toString(),
                sameBatch: snapshot(receiveUpdates(state, [...over, ...updates]).state),
                afterPrefix: snapshot(receiveUpdates(received, updates).state),
            };
        }
        console.log(JSON.stringify({
            name: 'effect acknowledgement in split-wire prefix', source, version, local, prefix,
            initial: snapshot(state), authority: authority.toString(), received: snapshot(received),
            concrete: replay(concrete), described: replay(described),
        }));
    }

    {
        const source = 'abc';
        const type = StateEffect.define({map([name, pos], changes) {
            const mapped = changes.mapPos(pos, 1, MapMode.TrackDel);
            return mapped === null ? undefined : [name, mapped];
        }});
        const local = [{changes: [[1], 2], marks: []}, {changes: [[1], 1], marks: []}];
        const inbound = {client: 'b', changes: [3], marks: [['left', 0], ['join', 1], ['right', 2]]};
        const state = applyLocal(start(source), local, type);
        const update = decode(inbound, type);
        const receipt = receiveUpdates(state, [update]);
        const stepwise = sendableUpdates(state).reduce(
            (effects, pending) => StateEffect.mapEffects(effects, pending.changes), update.effects,
        );
        console.log(JSON.stringify({
            name: 'inbound TrackDel through two front deletes', source, version, local, inbound,
            initial: snapshot(state), received: snapshot(receipt.state), stepwise: values(stepwise),
            receipt: {
                changes: receipt.changes.toJSON(), marks: values(receipt.effects),
                remote: receipt.annotation(Transaction.remote), history: receipt.annotation(Transaction.addToHistory),
            },
        }));
    }
} finally {
    rmSync(root, {recursive: true, force: true});
}
