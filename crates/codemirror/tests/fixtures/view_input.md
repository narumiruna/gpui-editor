# Input handler browser fixture

`view_input.jsonl` records headless Google Chrome 158.0.8127.37. The SHA-512 digest of each of these npm archives matches `docs/codemirror-upstream.json`. The loaded `dist/index.js` files match the archive entries byte for byte.

- `@codemirror/view` 6.44.0
- `@codemirror/state` 6.7.6
- `@codemirror/commands` 6.11.2

The driver mounts actual editors and verifies that they have focus. Chrome performs `insertText` and `delete` editing commands on the editable content and emits trusted `input` events. The driver checks `event.isTrusted` and does not dispatch a fabricated input event or mutate the editor DOM. The data is synthetic. The driver does not access the clipboard, remote pages, or an existing browser profile.

Each result records the handler order and arguments, the previews of the default transaction, the dispatches, and the resulting document, selection, and undo depth. The driver checks that repeated requests from different handlers return the same transaction in one operation. It checks that dispatch accepts that exact object when it uses the default. Browser offsets count UTF-16 code units. The Rust test converts them to UTF-8 byte offsets and dispatches only the transaction returned by the planner.

## Native scope

- `applyDOMChange` finds the replacement by comparing DOM text with the old document. It can detect a type-over of the selection, even if the text is unchanged. A selection-only event does not call input handlers.
- `applyDefaultInsert` can extend an ordinary non-composing change inside the primary selection to fill that selection and repeat it at other selections. The change must cover at least one third of the old selection. The new selection must be absent or an empty range after the insert. The default keeps the unchanged prefix and suffix of the primary selection. Other branches can target a position next to an atomic range, use the DOM's new primary selection, or map the old selection.
- During composition, the browser can repeat a change at other cursors with matching old text. It avoids editing cursors in the composition node because this can abort composition. The view also uses composition state and timing to label `input.type.compose` and `input.type.compose.start`. These browser decisions are not fixture or native planner requirements. The fixture tests ordinary input only.
- GPUI's `replace_text_in_range` uses an explicit UTF-16 range, then a marked range, then the current selection. GPUI owns UTF-16 conversion, composition grouping, and other typing behavior. A host can supply its own lazy transaction with `run_input_handlers` and continue its existing path on `Pass`. The native planner does not detect input from browser events, infer these selection or IME rules, or apply edits or effects.

## Regenerate

Run from the repository root. Use the scratch browser runner with the verified upstream package tree. It uses loopback and a temporary headless Chrome profile. Use only a trusted directory with no sensitive data.

```sh
fixture=crates/codemirror/tests/fixtures/view_input
root=$PWD
upstream=/private/tmp/gpui-upstream-exports.UVI6VVWZ
scratch=$(mktemp -d)
cp "$fixture.mjs" "$upstream/native-support-browser.mjs" "$scratch/"
ln -s "$upstream/node_modules" "$scratch/node_modules"
(cd "$scratch" && node native-support-browser.mjs view_input.mjs) > "$root/$fixture.jsonl"
```
