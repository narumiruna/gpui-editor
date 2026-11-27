// node crates/codemirror/tests/fixtures/change_wire_oracle.mjs <archive-root>
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {pinnedModules} from './pinned_modules.mjs';

const root = realpathSync(mkdtempSync(join(tmpdir(), 'change-wire-')));
try {
    const {load} = pinnedModules(resolve(process.argv[2]), root);
    const {ChangeSet, EditorState, StateEffect, Text} = await load('@codemirror/state');
    const {collab, getSyncedVersion, rebaseUpdates, receiveUpdates, sendableUpdates} =
        await load('@codemirror/collab');
    const wire = [1, 3], deletion = [[1], [1], 2];
    const sets = [
        ['abcd', wire],
        ['abcdef', [1, 1, [1, 'X'], 1, 2]],
        ['abcd', deletion],
        ['abcd', [1, 1, [0, 'X', ''], [0, 'Y'], 1, 1]],
    ].map(([doc, input]) => {
        const source = Text.of([doc]);
        const set = ChangeSet.fromJSON(input);
        const gaps = [];
        set.iterGaps((old, next, len) => gaps.push([old, next, len]));
        return {doc, input, wire: set.toJSON(), desc: set.desc.toJSON(), gaps,
            empty: set.empty, applied: set.apply(source).toString(),
            inverse: set.invert(source).toJSON(),
            spec: EditorState.create({doc}).changes(set).toJSON()};
    });
    const split = ChangeSet.fromJSON(deletion);
    const transform = {
        map: split.map(ChangeSet.fromJSON(wire)).toJSON(),
        compose: split.compose(ChangeSet.fromJSON([1, 1])).toJSON(),
    };
    const mark = StateEffect.define({map(value, desc) {
        const geometry = desc instanceof ChangeSet ? desc.desc : desc;
        return [...value, [geometry.empty, geometry.toJSON()]];
    }});
    const effects = items => items.map(item => item.value);
    const queued = updates => updates.map(update => [update.changes.toJSON(), effects(update.effects)]);
    const cases = [
        {doc: 'abcd', remote: wire, local: null},
        {doc: 'abcd', remote: wire, local: [4]},
        {doc: 'abcd', remote: wire, local: deletion},
        {doc: 'abcdef', remote: sets[1].wire, local: [6]},
    ].map(({doc, remote, local}) => {
        let state = EditorState.create({doc, extensions: collab({
            clientID: 'a', startVersion: 7, sharedEffects: tr => tr.effects
        })});
        if (local) state = state.update({changes: ChangeSet.fromJSON(local), effects: mark.of([])}).state;
        const second = ChangeSet.fromJSON(remote);
        const accepted = [
            {clientID: 'b', changes: ChangeSet.empty(doc.length), effects: [mark.of([])]},
            {clientID: 'c', changes: second, effects: []},
        ];
        const pending = sendableUpdates(state);
        const receipt = receiveUpdates(state, accepted);
        const full = rebaseUpdates(pending, accepted);
        const described = rebaseUpdates(pending, accepted.map(u => ({...u, changes: u.changes.desc})));
        return {doc, remote, local, desc: second.desc.toJSON(),
            receipt: {wire: receipt.changes.toJSON(), effects: effects(receipt.effects),
                doc: receipt.newDoc.toString(), version: getSyncedVersion(receipt.state),
                pending: queued(sendableUpdates(receipt.state))},
            full: queued(full), described: queued(described)};
    });
    console.log(JSON.stringify({sets, transform, cases}));
} finally {
    rmSync(root, {recursive: true, force: true});
}
