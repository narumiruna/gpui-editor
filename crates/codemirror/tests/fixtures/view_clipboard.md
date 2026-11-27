# Clipboard browser fixture

`view_clipboard.jsonl` records headless Google Chrome 158.0.8127.37. The SHA-512 digest of each npm archive below matches `docs/codemirror-upstream.json`. The browser loaded `dist/index.js` files that match their archive entries byte for byte.

- `@codemirror/view` 6.44.0: `sha512-kAUekx35l9u64jW/NzdO054l80lz+ri+Z5ZFS+dMpMrCbG5pFiTz58twWcWghzBEInWno0SPaURNSCnHHvipnQ==`
- `@codemirror/state` 6.7.6: `sha512-kAz+AncRtKuIknedxT1bq4XwXv4UowhbkHU1myPrtVb/jZtImWuV5BXzv5vK6i3kYACsdiZiQKFQQ5Mq7elW8w==`

The driver focuses actual editor instances in the browser. It sends synthetic `ClipboardEvent` events with a new `DataTransfer` for each operation. It does not read or write the OS clipboard. Each row resets CodeMirror's linewise marker with an ordinary copy. The results include the filter inputs, transaction metadata, document text, and selections. Selection positions are UTF-16 code units. The Rust test converts these to native UTF-8 byte offsets.

To regenerate, use the pinned upstream scratch directory with the verified npm packages in `node_modules` and the `native-support-browser.mjs` runner. This runner serves the driver on loopback and uses headless Chrome with a temporary profile. Use a trusted scratch directory that contains no sensitive files.

```sh
fixture=crates/codemirror/tests/fixtures/view_clipboard
root=$PWD
upstream=/private/tmp/gpui-upstream-exports.UVI6VVWZ
scratch=$(mktemp -d)
cp "$fixture.mjs" "$upstream/native-support-browser.mjs" "$scratch/"
ln -s "$upstream/node_modules" "$scratch/node_modules"
(cd "$scratch" && node native-support-browser.mjs view_clipboard.mjs) > "$root/$fixture.jsonl"
```

The native planner uses `EditorState::to_text`, which still treats unmatched LF as a line break when a custom separator is configured. Upstream can keep that LF inside a logical line. Native raw text can also have two-byte CRLF boundaries, which a linewise cut deletes in full. The browser fixture does not test transport details such as MIME fallback, DOM selection ownership, or drag and drop.
