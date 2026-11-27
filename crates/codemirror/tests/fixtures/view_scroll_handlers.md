# CodeMirror scroll interception oracle

`view_scroll_handlers.jsonl` records Google Chrome 158.0.8127.37 in headless mode. The browser loaded `@codemirror/view` 6.44.0 and `@codemirror/state` 6.7.6. The SHA-512 of each npm archive matches `docs/codemirror-upstream.json`. Each loaded `dist/index.js` matches its archive entry.

The driver dispatches reveal and restore effects to real editors in fixed, scaled scrollers. It records handler, exception sink, caret lookup, and covered-edge provider calls in order. `assigned` records the offsets that CodeMirror requests. The driver forwards those assignments unchanged; `actual` records Chrome's offsets after rounding and clamping. The legacy inset case records a separate reference rectangle: the bounding box of the scroll parent intersected with the window, not the client rectangle used for reveal.

To regenerate, serve the driver as a browser module with the verified upstream import map. Once it completes, write each entry of `globalThis.nativeSupport` as a JSON line. Run these commands from the repository root to use the scratch browser runner:

```sh
root=$PWD
upstream=/private/tmp/gpui-upstream-exports.UVI6VVWZ
fixture=crates/codemirror/tests/fixtures/view_scroll_handlers
cp "$fixture.mjs" "$upstream/view-scroll-handlers-oracle.mjs"
(cd "$upstream" && node native-support-browser.mjs view-scroll-handlers-oracle.mjs) > "$root/$fixture.jsonl"
```
