// Copy into the module root containing the verified tarballs' packages, then
// run with Node. Requires @codemirror/collab 6.1.1 and @codemirror/state 6.7.6.
import * as cm from '@codemirror/collab';
import {ChangeSet, EditorState, StateEffect} from '@codemirror/state';

function wire(changes, doc) {
  const parts = [];
  const bytes = (from, to) => Buffer.byteLength(doc.sliceString(from, to));
  let end = 0;
  changes.iterChanges((from, to, _fromB, _toB, insert) => {
    if (end < from) parts.push(bytes(end, from));
    parts.push([bytes(from, to), ...(insert.length ? insert.toJSON() : [])]);
    end = to;
  }, true);
  if (end < doc.length) parts.push(bytes(end, doc.length));
  return parts;
}
const mark = StateEffect.define({map(value, desc) {
  return desc.mapPos(value, 1);
}});
const cases = [
  {
    name: 'split insertions in the first update',
    first: [[0, '🦊'], [0, 'B'], 1],
    second: [2, [0, '語'], [2]],
    normalized: [[[0, '🦊B'], 1], [2, [0, '語'], [2]]],
    localAt: 0,
  },
  {
    name: 'split deletions in the second update',
    first: [1, [0, 'AB']],
    second: [[0, '🦊'], [1], [1], 1],
    normalized: [[1, [0, 'AB']], [[0, '🦊'], [2], 1]],
    localAt: 1,
  },
];
for (const {name, first, second, normalized, localAt} of cases) {
  const source = 'é';
  const start = EditorState.create({doc: source, extensions: cm.collab({
    clientID: 'a', startVersion: 7, sharedEffects: tr => tr.effects,
  })});
  const local = start.update({changes: {from: localAt, insert: '!'}, effects: mark.of(1)}).state;
  const [pending] = cm.sendableUpdates(local);
  function decode(remote) {
    return remote.map((json, index) => ({changes: ChangeSet.fromJSON(json), clientID: ['b', 'c'][index]}));
  }
  const over = decode([first, second]);
  const authority = over.reduce((doc, u) => u.changes.apply(doc), start.doc);
  const receipt = cm.receiveUpdates(local, over);
  const [echo] = cm.rebaseUpdates([pending], over);
  const [described] = cm.rebaseUpdates([pending], over.map(u => ({...u, changes: u.changes.desc})));
  const [other] = cm.rebaseUpdates([pending], decode(normalized));
  const after = cm.receiveUpdates(local, [...over, echo]).state;
  const wrong = cm.receiveUpdates(local, [...over, other]).state;
  console.log(JSON.stringify({
    name, source, version: cm.getSyncedVersion(start),
    local: wire(pending.changes, start.doc), localMark: Buffer.byteLength(local.doc.sliceString(0, 1)),
    remote: over.map((u, index) => wire(u.changes, index ? over[0].changes.apply(start.doc) : start.doc)),
    authority: authority.toString(),
    receipt: wire(receipt.changes, local.doc),
    echo: wire(echo.changes, authority),
    described: wire(described.changes, authority),
    mark: Buffer.byteLength(receipt.newDoc.sliceString(0, echo.effects[0].value)),
    doc: after.doc.toString(), finalVersion: cm.getSyncedVersion(after),
    other: wire(other.changes, authority), otherAuthority: other.changes.apply(authority).toString(),
    wrongUpstream: {doc: wrong.doc.toString(), version: cm.getSyncedVersion(wrong), pending: cm.sendableUpdates(wrong).length},
  }));
}
