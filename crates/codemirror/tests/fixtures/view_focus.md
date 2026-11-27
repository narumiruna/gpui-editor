# CodeMirror focus oracle

`view_focus.jsonl` records real focus and blur in headless Google Chrome 158.0.8127.37. The SHA-512 digests of these npm archives match `docs/codemirror-upstream.json`. Each loaded `dist/index.js` also matches its archive entry byte for byte:

| Package | Version | SHA-512 integrity |
| --- | --- | --- |
| `@codemirror/view` | `6.44.0` | `sha512-kAUekx35l9u64jW/NzdO054l80lz+ri+Z5ZFS+dMpMrCbG5pFiTz58twWcWghzBEInWno0SPaURNSCnHHvipnQ==` |
| `@codemirror/state` | `6.7.6` | `sha512-kAz+AncRtKuIknedxT1bq4XwXv4UowhbkHU1myPrtVb/jZtImWuV5BXzv5vK6i3kYACsdiZiQKFQQ5Mq7elW8w==` |
| `@codemirror/commands` | `6.11.2` | `sha512-NbH6Q9lmcXvf9s5/mMJIwL2Hjl2NEdsN/dDlezGnaFOQgNZ59dT7ndKUf21/PIjJacD/Ti2Ifi4TsBkO6WHvsg==` |

The driver records provider calls, transaction batches, accepted effects, view plugin calls, and listener calls. It omits measurement updates with no transactions or focus change. The positions use ASCII input, so browser UTF-16 positions equal native byte offsets.

## Ordering and native boundaries

- `focusChangeTransaction` calls each provider in facet precedence order with the same state. It skips `null` and keeps typed effects with false values. An effect-bearing transaction has an internal focus annotation. It has no explicit selection, scroll request, user event, or history override.
- `updateForFocusChange` samples focus after a DOM timer. It ignores a transient focus-and-blur pair and a repeated sample. It dispatches a focus transaction when providers return effects. Otherwise it calls `update([])`. That update calls the providers again before it publishes a focus flag. The native host supplies one settled sample and does not repeat providers for an empty update.
- In an ordinary browser update, the view resolves the whole batch before it reads the new configuration and runs providers. A dispatch of multiple specs composes and maps their effects before that read. A dispatch of a transaction array retains the effects of each transaction in its own coordinates. With a focus transaction, observers see the ordinary update without a focus flag, then the separate focus update. Without a focus transaction, the ordinary update carries the focus flag. If an observer or a synchronous caller changes the state before the queued focus dispatch, the browser drops the stale transaction.
  - The native host reports focus separately after any preceding edits. There is no ambient DOM focus sample, transaction batch, or focus microtask in this API. The builder reads the state supplied by the caller. Dispatch a proposal against that state, not against a later state or merged into the preceding edit.
- The browser reports focus when a filter removes an effect but keeps the internal annotation. The native `set_focus` operation knows the reason for the update even if a filter replaces the complete proposal. It publishes focus only after the transaction succeeds and reports the accepted effects. An effect-only transaction makes an undo entry only if history obtains an inverse effect. A replay does not publish a focus edge.

## Regenerate the fixture

Serve `view_focus.mjs` as a browser module with an import map for the verified package tree. Read `globalThis.nativeSupport` after the module completes and write one entry per line. With the scratch browser runner, run these commands from the repository root:

```sh
root=$PWD
upstream=/private/tmp/gpui-upstream-exports.UVI6VVWZ
fixture=crates/codemirror/tests/fixtures/view_focus
cp "$fixture.mjs" "$upstream/view-focus-oracle.mjs"
(cd "$upstream" && node native-support-browser.mjs view-focus-oracle.mjs) > "$root/$fixture.jsonl"
```
