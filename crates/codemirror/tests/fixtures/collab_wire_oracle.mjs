// Run from a directory with the SHA-512-verified collab 6.1.1 and state 6.7.6
// packages installed. Copy this script there before running it with Node.
import * as cm from '@codemirror/collab';
import {ChangeSet, EditorState, MapMode, StateEffect, Transaction} from '@codemirror/state';

const doc = 'é🦊z';
const version = 8;
const mark = StateEffect.define({map(value, desc) {
  const pos = desc.mapPos(value[1], 1, MapMode.TrackDel);
  return pos === null ? undefined : [value[0], pos];
}});
const start = () => EditorState.create({doc, extensions: cm.collab({
  clientID: 'a', startVersion: version, sharedEffects: tr => tr.effects.filter(e => e.is(mark))
})});
const pending = start().update({effects: mark.of(['local', 1])}).state;
const foreign = {clientID: 'b', changes: ChangeSet.empty(doc.length), effects: [mark.of(['remote', 1])]};
const marks = effects => (effects ?? []).map(effect => effect.value);
const receipt = tr => ({
  doc: tr.newDoc.toString(),
  version: cm.getSyncedVersion(tr.state),
  client: cm.getClientID(tr.state),
  remote: tr.annotation(Transaction.remote) === true,
  history: tr.annotation(Transaction.addToHistory) !== false,
  marks: marks(tr.effects),
  pending: cm.sendableUpdates(tr.state).map(update => marks(update.effects))
});
const cases = [
  {utf16: [[1], [2], 1], bytes: [[2], [4], 1], description: [2, 0, 4, 0, 1, -1]},
  {utf16: [[3], 1], bytes: [[6], 1], description: [6, 0, 1, -1]}
].map(({utf16, ...input}) => {
  const over = [foreign, {clientID: 'c', changes: ChangeSet.fromJSON(utf16)}];
  const queued = cm.sendableUpdates(pending);
  return {...input, receipt: receipt(cm.receiveUpdates(pending, over)),
    full: cm.rebaseUpdates(queued, over).map(update => marks(update.effects)),
    described: cm.rebaseUpdates(queued, over.map(u => ({...u, changes: u.changes.desc})))
      .map(update => marks(update.effects))};
});
const split = ChangeSet.fromJSON([[1], [2], 1]);
let local = start().update({changes: split}).state;
local = local.update({changes: {from: 1, insert: '!'}}).state;
const own = cm.sendableUpdates(local);
const gap = {clientID: 'b', changes: ChangeSet.of({from: 1, insert: '?'}, 1)};
const [lastEcho] = cm.rebaseUpdates(own, [own[0], gap]);
const matching = [own[0], gap, lastEcho];
const contradictory = [{...own[0], changes: ChangeSet.fromJSON([[3], 1])}, gap, lastEcho];
const echoes = [matching, contradictory].map(over => ({
  receipt: receipt(cm.receiveUpdates(local, over)),
  full: cm.rebaseUpdates(own, over).length,
  described: cm.rebaseUpdates(own, over.map(u => ({...u, changes: u.changes.desc}))).length
}));
console.log(JSON.stringify({doc, version, position: Buffer.byteLength('é'), cases, echoes}));
