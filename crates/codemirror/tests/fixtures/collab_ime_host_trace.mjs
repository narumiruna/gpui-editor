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
const scratch = mkdtempSync(join(realpathSync(tmpdir()), 'gpui-collab-ime.'));
try {
    const {load} = pinnedModules(resolve(process.argv[2]), scratch);
    const {collab, getSyncedVersion, receiveUpdates, sendableUpdates} = await load('@codemirror/collab');
    const {history, isolateHistory, undo, redo, undoDepth, redoDepth} = await load('@codemirror/commands');
    const {ChangeSet, EditorState} = await load('@codemirror/state');
    const create = clientID => EditorState.create({doc: source, extensions: [
        collab({clientID, startVersion: base}), history(), EditorState.lineSeparator.of(separator),
    ]});
    function atEnd(state, insert) {
        const at = state.doc.length;
        return state.update({changes: {from: at, insert}, selection: {anchor: at + insert.length}, userEvent: 'input.type'}).state;
    }
    let a = atEnd(create('a'), '!');
    let b = atEnd(create('b'), '🦊');
    let authority = create('authority').doc;
    const accepted = [];
    const submit = state => {
        assert.equal(getSyncedVersion(state), base + accepted.length);
        for (const {changes, clientID} of sendableUpdates(state)) {
            authority = changes.apply(authority);
            accepted.push({changes, clientID});
        }
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

    // The native host sends one composed edit, not these marked revisions.
    const at = a.doc.length;
    let staged = ChangeSet.empty(at);
    let marked = '';
    for (const text of ['ni', '你🦀', '你好']) {
        staged = staged.compose(ChangeSet.of({from: at, to: at + marked.length, insert: text}, staged.newLength));
        marked = text;
    }
    a = a.update({changes: staged, selection: {anchor: at + marked.length}, userEvent: 'input.type.compose.start', annotations: isolateHistory.of('full')}).state;
    assert.deepEqual(sendableUpdates(a).map(update => update.changes.toJSON()), [[at - 1, [0, '!']], [at, [0, '你好']]]);
    submit(b);
    a = deliver(a);
    b = deliver(b);
    submit(a);
    a = deliver(a);
    b = deliver(b);
    record('committed');
    for (const [name, action] of [['undo', undo], ['redo', redo]]) {
        assert(action({state: a, dispatch(tr) { a = tr.state; }}));
        submit(a);
        a = deliver(a);
        b = deliver(b);
        record(name);
    }
    console.log(JSON.stringify({source, base, stages}));
} finally {
    rmSync(scratch, {recursive: true, force: true});
}
