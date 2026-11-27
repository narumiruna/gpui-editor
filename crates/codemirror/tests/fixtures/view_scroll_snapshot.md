# CodeMirror scroll oracle

`view_scroll_snapshot.jsonl` records headless Google Chrome 158.0.8127.37 with `@codemirror/view` 6.44.0 and `@codemirror/state` 6.7.6. The SHA-512 of each npm archive matches `docs/codemirror-upstream.json`. Each loaded `dist/index.js` matches its archive entry.

The browser driver, `view_scroll_snapshot.mjs`, mounts real editors. It captures snapshots with `EditorView.scrollSnapshot()`, maps them through real changes, and restores them through a transaction or the constructor's `scrollTo` option. For reveal, it records caret rectangles and intercepts assignments from the pinned `DocView.scrollIntoView` method. It forwards each assignment to Chrome unchanged. `assigned` contains the requested offsets, and `actual` contains the offsets after browser rounding and clamping. All document positions in the fixture are UTF-8 byte offsets.

To regenerate, serve the driver as a browser module with an import map for the verified package tree. Read `globalThis.nativeSupport` after the module completes and write the array entries as JSON lines. With the scratch browser runner used to generate the fixture, run these commands from the repository root:

```sh
root=$PWD
upstream=/private/tmp/gpui-upstream-exports.UVI6VVWZ
fixture=crates/codemirror/tests/fixtures/view_scroll_snapshot
cp "$fixture.mjs" "$upstream/view-scroll-snapshot-oracle.mjs"
(cd "$upstream" && node native-support-browser.mjs view-scroll-snapshot-oracle.mjs) > "$root/$fixture.jsonl"
```
