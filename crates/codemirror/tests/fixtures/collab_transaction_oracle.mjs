// Replay with Node and the directory of SHA-512-pinned archives.
// Uses EditorState and headless history commands, not a DOM.
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {pinnedModules} from './pinned_modules.mjs';

const scratch = mkdtempSync(join(realpathSync(tmpdir()), 'gpui-collab-transactions.'));
try {
  const {load} = pinnedModules(resolve(process.argv[2]), scratch);
  const cm = await load('@codemirror/collab');
  const {history, redo, redoDepth, undo, undoDepth} = await load('@codemirror/commands');
  const {ChangeSet, EditorState, StateEffect, Transaction} = await load('@codemirror/state');

  const effect = StateEffect.define();
  let hooks = [0, 0, 0, 0]; // change filter, transaction filter, extender, sharing
  let state = EditorState.create({doc: 'abcd', extensions: [
    cm.collab({clientID: 'a', startVersion: 7, sharedEffects(tr) {
      hooks[3]++;
      return tr.effects.filter(e => e.is(effect));
    }}),
    history(),
    EditorState.changeFilter.of(tr => {
      hooks[0]++;
      return tr.isUserEvent('input') ? [1, 3] : true;
    }),
    EditorState.transactionFilter.of(tr => { hooks[1]++; return tr; }),
    EditorState.transactionExtender.of(() => { hooks[2]++; return {effects: effect.of('extended')}; }),
  ]});
  let stages = [];
  function step(name, apply) {
    hooks = [0, 0, 0, 0];
    const tr = apply();
    state = tr.state;
    stages.push({name, hooks, doc: state.doc.toString(), version: cm.getSyncedVersion(state),
      remote: tr.annotation(Transaction.remote) === true,
      history: tr.annotation(Transaction.addToHistory) !== false,
      depth: [undoDepth(state), redoDepth(state)],
      pending: cm.sendableUpdates(state).map(u => [u.changes.toJSON(),
        u.origin.annotation(Transaction.userEvent), u.effects.map(e => e.value)]),
    });
  }
  function run(command) {
    let transaction;
    if (!command({state, dispatch: tr => { transaction = tr; }})) throw new Error('missing history event');
    return transaction;
  }
  step('local', () => state.update({changes: {from: 0, to: 4, insert: '!'}, userEvent: 'input.type'}));
  step('remote', () => cm.receiveUpdates(state, [{clientID: 'b', changes: ChangeSet.of({from: 1, to: 2, insert: 'R'}, 4)}]));
  step('undo', () => run(undo));
  step('ack', () => cm.receiveUpdates(state, cm.sendableUpdates(state)));
  step('redo', () => run(redo));
  step('empty', () => cm.receiveUpdates(state, []));
  console.log(JSON.stringify(stages));
} finally {
  rmSync(scratch, {recursive: true, force: true});
}
