# GPUI Editor CodeMirror core

`gpui-editor-codemirror` provides text, selections, transactions, and undo history for a native editor. It does not depend on GPUI, a browser, or JavaScript. It implements part of the CodeMirror 6 state model, not the full JavaScript API.

## Edit and history rules

- All offsets are UTF-8 bytes. Edits must start and end at character boundaries. `Text` keeps LF and CRLF bytes as given. Logical line ranges omit their line endings.
- A change set contains non-overlapping replacements in the old document. A transaction can set a selection in the new document. If it does not, the transaction maps the old selection. An invalid edit or selection leaves the state unchanged.
- A cursor stays before text inserted at its position. For a nonempty range, the start maps after an insertion at the start, and the end maps before an insertion at the end. The range keeps its direction. If a replacement strictly covers both endpoints, Rust collapses the range to the start of the replacement. Upstream can map the endpoints across each other in this case.
- History stores change sets and their inverses, with the before and after selections and caller metadata. It keeps up to 100 undo groups. Outside an explicit group, each text edit creates one undo step. Selection-only updates do not enter history. A new text edit clears redo.

## CodeMirror 6 reference

The [Rust fixtures](tests/upstream.rs) use independent inputs checked against these exact npm releases:

| Package | npm `gitHead` | Upstream tests inspected |
| --- | --- | --- |
| [`@codemirror/state` 6.7.6](https://registry.npmjs.org/@codemirror/state/-/state-6.7.6.tgz) | `651084c8a322b3a19b2bb3e00eee7d4a61d641af` | `test/test-change.ts`, `test/test-selection.ts`, `test/test-state.ts` |
| [`@codemirror/commands` 6.11.2](https://registry.npmjs.org/@codemirror/commands/-/commands-6.11.2.tgz) | `2d2511920a7201ba65e3a9d86e5bd941aef8dc25` | `test/test-history.ts` |

To check the reference, install the two releases in a temporary directory:

```sh
npm install --ignore-scripts --save-exact @codemirror/state@6.7.6 @codemirror/commands@6.11.2
```

Use `ChangeSet.of` and `mapPos` with association `-1` or `+1`, `EditorSelection.create`, and `EditorState.update`. Upstream positions count UTF-16 code units. To convert an upstream position at a character boundary, count the UTF-8 bytes in the document prefix up to that position. Use the old document for input positions and the new document for mapped positions and explicit selections.

Upstream also maps empty ranges with association `-1` by default, regardless of the stored cursor association. Both cores keep touching nonempty ranges separate. Rust rejects overlapping changes rather than composing them, and it keeps the original line endings instead of converting mixed line endings to LF.

The upstream commands `history`, `undo`, and `redo` replay the two adjacent typing edits in the fixture as separate steps when their `Transaction.time` values differ by 500 ms. At 499 ms they form one step. Isolation, nonadjacent edits, an intervening selection update, and paste also separated groups in the cases checked. Rust does not group by time, edit type, or adjacency. The caller sets group boundaries explicitly, for example for a series of input method updates.
