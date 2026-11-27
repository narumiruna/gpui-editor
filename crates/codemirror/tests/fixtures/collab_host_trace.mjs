// Replay with Node and the directory of SHA-512-pinned archives.
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {pinnedModules} from './pinned_modules.mjs';

const prefix = '# 🧭 raw\r\n\r\n[link](https://example.invalid) `literal`\r\n\r\n~~~rs  \r\n';
const source = prefix + 'éz\rraw';
const base = 19;
const separator = '\r\n';
const scratch = mkdtempSync(join(realpathSync(tmpdir()), 'gpui-collab-host.'));
try {
    const {load} = pinnedModules(resolve(process.argv[2]), scratch);
    const {collab, getSyncedVersion, receiveUpdates, sendableUpdates} = await load('@codemirror/collab');
    const {history, undo, redo, undoDepth, redoDepth} = await load('@codemirror/commands');
    const {EditorState} = await load('@codemirror/state');
    const create = clientID => {
        const state = EditorState.create({doc: source, extensions: [
            collab({clientID, startVersion: base}), history(), EditorState.lineSeparator.of(separator),
        ]});
        return state.update({selection: {anchor: state.doc.length}}).state;
    };
    const insertAtEnd = (state, insert) => {
        const at = state.doc.length;
        return state.update({changes: {from: at, insert}, selection: {anchor: at + insert.length}, userEvent: 'input.type'}).state;
    };
    let a = insertAtEnd(create('a'), '🐈');
    let b = insertAtEnd(create('b'), '🦊');
    let authority = create('authority').doc;
    const accepted = [];
    const submit = state => {
        if (getSyncedVersion(state) !== base + accepted.length) return false;
        const batch = sendableUpdates(state).map(({changes, clientID}) => ({changes, clientID}));
        for (const {changes} of batch) authority = changes.apply(authority);
        accepted.push(...batch);
        return true;
    };
    const deliver = state => receiveUpdates(state, accepted.slice(getSyncedVersion(state) - base)).state;
    const raw = doc => doc.sliceString(0, doc.length, separator);
    const stages = [];
    const record = name => {
        for (const peer of [a, b]) {
            assert.equal(raw(peer.doc), raw(authority));
            assert.equal(getSyncedVersion(peer), base + accepted.length);
            assert.equal(sendableUpdates(peer).length, 0);
        }
        stages.push({name, draft: raw(authority).slice(prefix.length), version: getSyncedVersion(a),
            caret: Buffer.byteLength(a.sliceDoc(0, a.selection.main.head)), depths: [undoDepth(a), redoDepth(a)]});
    };
    assert(submit(b));
    assert(!submit(a));
    a = deliver(a);
    b = deliver(b);
    assert.deepEqual(sendableUpdates(a).map(({clientID, changes}) => [clientID, changes.toJSON()]), [
        ['a', [authority.length, [0, '🐈']]],
    ]);
    assert(submit(a));
    a = deliver(a);
    b = deliver(b);
    record('synced');
    for (const [name, command] of [['undo', undo], ['redo', redo]]) {
        assert(command({state: a, dispatch(tr) { a = tr.state; }}));
        assert(submit(a));
        a = deliver(a);
        b = deliver(b);
        record(name);
    }
    a = insertAtEnd(a, 'λ');
    assert(submit(a));
    a = deliver(a);
    b = deliver(b);
    record('next-input');
    console.log(JSON.stringify({source, base, stages}));
} finally {
    rmSync(scratch, {recursive: true, force: true});
}
