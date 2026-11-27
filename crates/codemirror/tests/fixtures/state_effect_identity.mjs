// node crates/codemirror/tests/fixtures/state_effect_identity.mjs <archive-root>
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {pinnedModules} from './pinned_modules.mjs';

const root = realpathSync(mkdtempSync(join(tmpdir(), 'state-effect-identity-')));
try {
    const {load} = pinnedModules(resolve(process.argv[2]), root);
    const {ChangeDesc, ChangeSet, StateEffect} = await load('@codemirror/state');
    const {rebaseUpdates} = await load('@codemirror/collab');
    for (const [name, wire] of [
        ['empty document', []],
        ['canonical identity', [4, -1]],
        ['split identity', [1, -1, 3, -1]],
        ['split deletion', [1, 0, 1, 0, 2, -1]],
    ]) {
        const desc = ChangeDesc.fromJSON(wire);
        const calls = [];
        const type = StateEffect.define({map(value, mapping) {
            calls.push([value, mapping.empty, mapping.toJSON()]);
            return value === 'drop' ? undefined : `${value}:${mapping.empty}`;
        }});
        const effects = ['first', 'drop', 'last'].map(value => type.of(value));
        const values = StateEffect.mapEffects(effects, desc).map(effect => effect.value);
        const directCalls = calls.splice(0);
        const [update] = rebaseUpdates([{clientID: 'a', changes: ChangeSet.empty(desc.length), effects}],
            [{clientID: 'b', changes: desc}]);
        const identity = ChangeSet.empty(desc.length);
        const later = ChangeDesc.fromJSON(desc.newLength ? [desc.newLength, -1] : []);
        // Decode with an actual set so adjacent deletions remain individually
        // visible before a forced map.
        const edits = desc.length ? ChangeSet.fromJSON([[1], [1], 2]) : identity;
        const editsLater = ChangeSet.empty(edits.newLength);
        console.log(JSON.stringify({name, wire, empty: desc.empty, calls: directCalls, values,
            rebased: {calls, values: update.effects.map(effect => effect.value)},
            before: identity.composeDesc(desc).toJSON(), after: desc.composeDesc(later).toJSON(),
            mapped: desc.mapDesc(identity).toJSON(),
            edits: {input: edits.toJSON(),
                before: identity.compose(edits).toJSON(), after: edits.compose(editsLater).toJSON(),
                mapped: edits.map(identity).toJSON(), forced: edits.mapDesc(identity).toJSON()}}));
    }
} finally {
    rmSync(root, {recursive: true, force: true});
}
