# GPUI Editor

GPUI Editor is a local Markdown editor for macOS. It uses GPUI to show formatted text, images, links, Mermaid flowcharts, and LaTeX formulas in one editing surface. It saves Markdown without rewriting parts you did not edit.

## Build and run on macOS

Install Rust and Xcode. The native build was verified on Apple Silicon with macOS 26.4.1, Rust 1.95.0, and Xcode 26.6. Other configurations are not yet verified. GPUI compiles shaders at runtime, so the separate Metal compiler component is not required.

Install [Just](https://just.systems/) and run this from the repository root:

```sh
just run
```

This builds and starts the app. It uses `/Applications/Xcode.app` by default. If Xcode is installed elsewhere, set `DEVELOPER_DIR` to that installation's `Contents/Developer` directory before running `just run`.

The command takes no path. In the window, choose **Open Folder…**, then select a Markdown file from the sidebar. Use **New note** for a blank document and **Save** or `Cmd-S` to save. The document bar shows the filename and save status. The sidebar lists only the workspace's top-level `.md` files.

- Use the labeled toolbar to format a selection, insert a diagram, add an inline formula, or insert a link. **More tools** shows front matter, display formulas, images, and item actions. Click front matter, a diagram, or a formula to edit its source. Click outside the item to preview the change; this does not save the file.
- For links, hover to see the full destination in the bottom bar. Click to open the web browser or follow an in-workspace `.md` link. To edit a link without opening it, place the caret at its edge and choose **More tools** → **Edit link**.
- Image previews support local or HTTPS PNG, JPEG, GIF, and WebP still images. Relative paths are based on the document's folder. To fetch HTTPS previews, choose **Load remote images for this document**. The remote host receives the image request; redirects are not followed.
- The editor supports a subset of Markdown. Inline math cannot span paragraph breaks. The Mermaid renderer supports flat flowcharts; unsupported blocks such as GFM tables are shown as read-only source.
- Finish or cancel any active input method composition before saving, closing, or switching documents. Then try the action again.
- The editor stops a save if it detects an earlier external change. Existing files need a filesystem that supports atomic exchange. If a competing version appears during the exchange, the app reports a conflict and gives the path to the displaced version in a private `.gpui-save-*` folder. The draft may already be at the original path. Review both versions before continuing. If the editor reports that the folder moved during a first save, the draft may be in the moved folder. Check that folder before retrying. If a different folder takes the workspace path, select it again to switch workspaces.

LaTeX previews use STIX Two Math through Mathra. See the [STIX Two Math license](docs/licenses/stix-two-math.txt).

## Develop

The Cargo workspace separates the [app](crates/app/), [Markdown model](crates/document/), [previews](crates/preview/), and [file handling](crates/workspace/). App-only changes reuse the other compiled crates. The crates share the root `target/` directory and lockfile.

Run checks for a single component with `-p`. For example, this tests the Markdown model without building GPUI:

```sh
cargo test --locked -p gpui-editor-document
```

The app and its internal crates build from source as a workspace. They are not published separately to crates.io.

### Developer Mode and Pi

1. Start GPUI Editor with `just run`. In **GPUI Editor → Preferences…**, turn on **Developer Mode**. A target icon appears in the upper-right corner.
2. Start Pi 1.6.0 or later in this repository. Trust the project if you want Pi to load [its local extension](.pi/extensions/gpui-editor.ts). In Pi, run `/gpui-pair`. If Pi finds more than one editor, choose the one to pair. Verify the code in both apps, then choose **Pair** in GPUI Editor.
3. Click the target icon, then click an element such as **Save**, a sidebar row, or the editing surface. Picking does not run the element's usual action. Press `Esc` to leave picking mode.
4. Write your feedback. Choose **Capture Window** to attach a screenshot of GPUI Editor, or **Paste Image** to attach a PNG, JPEG, or TIFF image from the clipboard. If macOS denies Screen & System Audio Recording, use **Paste Image** or send text only. Review attachments before choosing **Send to Pi**. An image requires an image-capable Pi model.

Feedback goes to the paired session as a user message. If Pi is busy, it follows the current task. Pi may save feedback in its session history or send it to its model provider. A receipt confirms only that the extension handed feedback to Pi. If the connection drops before the receipt, check Pi before resending.

With a live pairing, Pi can call `gpui_editor_logs` to read a bounded history of editor diagnostics. It does not read documents, raw paths, or system logs. To disconnect, use `/gpui-unpair` in Pi, choose **Unpair Pi** in Preferences, or turn Developer Mode off. Pi session changes, reloads, and exits also disconnect. The editor starts with the mode off and does not reopen pairings. See [ADR 0002](docs/decisions/0002-local-pi-session-pairing.md) for the data boundaries.

## Checks

Install [Bun](https://bun.com/) to run the Pi extension tests.

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --all-targets
prek run --all-files
bun test ./.pi/tests/gpui-editor.test.ts
```

GitHub Actions runs Rust checks with Rust 1.95.0 on Linux and macOS. The Linux job checks the portable document, rendering, and file-handling code; only macOS builds the GPUI application. Automated GPUI tests use a simulated window, not the real macOS input method. If macOS asks the editor to close during logout or restart, the editor can protect unsaved changes, but the system request must be started again after the editor closes. See the [manual acceptance steps](docs/prd/gpui-editor.md#macos-原生驗收) for the remaining native checks.

[Changenote](https://github.com/changenote/changenote) uses [`.changenote/`](.changenote/) to maintain release notes and a release pull request. Changes to the editor do not require a manual release workflow run.
