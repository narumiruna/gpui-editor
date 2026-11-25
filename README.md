# GPUI Editor

GPUI Editor is a local Markdown editor for macOS. It uses GPUI to show formatted text, tables, code blocks, images, links, Mermaid flowcharts, and LaTeX formulas in one editing surface. It saves Markdown without rewriting parts you did not edit.

## Build and run on macOS

Install Rust and Xcode. The native build was verified on Apple Silicon with macOS 26.4.1, Rust 1.95.0, and Xcode 26.6. Other configurations are not yet verified. GPUI compiles shaders at runtime, so the separate Metal compiler component is not required.

Install [Just](https://just.systems/) and run this from the repository root:

```sh
just run
```

This builds and starts the app. It uses `/Applications/Xcode.app` by default. If Xcode is installed elsewhere, set `DEVELOPER_DIR` to that installation's `Contents/Developer` directory before running `just run`.

The command takes no path. If there is no workspace to restore, choose **Open Folder…** in the sidebar. Expand folders in the file tree to browse their contents. The tree shows ordinary files and folders without following symlinks. Click a Markdown file or follow a Markdown link within the workspace to open a tab or select its existing tab. **New note** opens a separate blank tab.

Drag the divider to resize the sidebar. Use the sidebar chevron, **View → Toggle Sidebar**, or `Cmd-Option-S` to hide or show it without changing its state or the active document tab. When the sidebar is hidden, **Open Folder…** and **New note** move to the editor toolbar. The macOS window controls share the top row; drag an empty part of that row to move the window.

### Find documents in the sidebar

The sidebar starts on **Tree**. Select a tab below the sidebar toolbar to show one view at a time. At narrow widths, hover over a tab icon to see its name. Drag a sidebar tab to reorder it. Switching or reordering sidebar tabs does not switch, save, or reorder document tabs.

After clicking a sidebar tab, use `Tab` or `Shift-Tab` to move between sidebar controls, `Enter` or `Space` to activate a focused button, and `Esc` to return to the document.

- **Tree:** Expand folders and click a Markdown file to open it. Hover over a Markdown row or give it keyboard focus to pin or unpin the file without opening it.
- **Recent:** Reopen one of the 50 most recently opened files. Opening a file again moves it to the top without adding a duplicate. Clicking an already open document tab, restoring a session, and saving a new file for the first time do not count as opens.
- **Pins:** Reopen pinned files even after closing their document tabs. Hover over a row or give it keyboard focus to remove its pin without closing or deleting the file. **Recent** and **Pins** show workspace-relative paths to distinguish files with the same name.
- **Outline:** Click a heading to scroll to it in the current document, including an unsaved or untitled draft. This does not move the caret or change the selection.
- **Search:** Type a single line and press `Enter` after finishing any input method composition. Search matches the exact text, including letter case, in UTF-8 `.md` filenames and contents saved on disk, not in drafts. Click a filename match to open the file, or a content match to open it and scroll to the match. A stale content match leaves your draft intact and asks you to search again instead of guessing where to jump. Search does not replace text.

The selected sidebar view, tab order, and width stay in the window when you hide the sidebar or switch workspaces. Recent files, pins, and the search query and results stay until you successfully switch to a different workspace. None of these sidebar settings or lists return on restart. Document tabs still restore as described below.

### Work with tabs

- Click a tab to switch without saving. Each open tab keeps its own draft, caret, and undo history. The tab bar shows the selected tab and marks unsaved changes. Scroll the tab bar if not all tabs fit. **Save** or `Cmd-S` saves only the selected tab.
- Use a tab's close button or `Cmd-W` to close a tab. `Cmd-W` targets the selected tab. When you close the selected tab, the editor selects the tab to its right, or the one to its left if there is none on the right. Closing the last tab leaves the workspace and sidebar open. With no tabs, `Cmd-W` closes the window.
- Closing a tab checks that tab for unsaved changes. Switching to a different workspace, using **Close Window** or the window close button, or quitting with `Cmd-Q` checks all tabs in order. For each draft, choose **Save**, **Don’t Save**, or **Cancel**. If you cancel or a save fails, the editor returns to the original tab. Changes already saved stay saved; the other drafts stay open. Choosing the same workspace again only refreshes the sidebar.
- Finish or cancel any active input method composition before saving, closing, or switching tabs. Then try the action again.

### Reopen the last workspace

After a normal exit, the next launch opens the last workspace and its saved tabs in order. It skips missing or unreadable files and selects the last selected tab if it can be restored, or the first available tab. If no tabs can be restored, it opens only the workspace. If the workspace is missing, inaccessible, or has been replaced, choose a folder again with **Open Folder…**. A missing or unreadable session record does not prevent manual selection, and the editor never opens the Terminal's current folder automatically.

The editor stores the folder and tab list locally in `~/Library/Application Support/GPUI Editor/session.json`, not in the workspace. It does not autosave documents or recover drafts after a crash. Standalone files saved outside a workspace, caret positions, undo history, remote image permissions, Developer Mode, and Pi pairings do not return on restart. If the editor cannot update the session record when closing, it leaves the previous record in place and offers **Keep Window Open** or **Quit Without Updating**.

### Edit Markdown

- Use the toolbar below the tabs to save, format a selection, insert a diagram, add an inline formula, or insert a link. Hover over an icon to see its name. At narrow window widths, scroll the toolbar to reach more actions. The three-dot **More tools** button shows front matter, display formulas, images, and item actions. Click front matter, a diagram, or a formula to edit its source. Click elsewhere in the document to preview the change; this does not save the file.
- For links, hover to see the full destination in a tooltip. Click to open the web browser or follow an in-workspace `.md` link. To edit a link without opening it, place the caret at its edge and choose **More tools** → **Edit link**.
- Image previews support local or HTTPS PNG, JPEG, GIF, and WebP still images. Relative paths are based on the document's folder. To fetch HTTPS previews, choose **Load remote images for this document** in the editor toolbar. The remote host receives the image request; redirects are not followed.
- GFM tables show a grid with a header and aligned plain-text cells. Ordinary fenced and indented code blocks show monospaced text without syntax highlighting. Both previews are read-only and scroll with the document. Prose wraps to fit the editor; wide previews can scroll horizontally. Click either preview to select the entire block; `Cmd-C` copies its original Markdown.
- The editor supports a subset of Markdown. Inline math cannot span paragraph breaks. The Mermaid renderer supports flat flowcharts; unsupported blocks such as block quotes are shown as read-only source.
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

1. Start GPUI Editor with `just run`. In **GPUI Editor → Preferences…**, turn on **Developer Mode**. A **Select element** button with a target icon appears in the upper-right corner.
2. Start Pi 1.6.0 or later in this repository. Trust the project if you want Pi to load [its local extension](.pi/extensions/gpui-editor.ts). In Pi, run `/gpui-pair`. If Pi finds more than one editor, choose the one to pair. Verify the code in both apps, then choose **Pair** in GPUI Editor.
3. Click **Select element**, then click an element such as **Save**, a sidebar row, or the editing surface. Picking does not run the element's usual action. Press `Esc` to leave picking mode.
4. Write your feedback. Choose **Capture Window** to attach a screenshot of GPUI Editor, or **Paste Image** to attach a PNG, JPEG, or TIFF image from the clipboard. If macOS denies Screen & System Audio Recording, use **Paste Image** or send text only. Review attachments before choosing **Send to Pi**. An image requires an image-capable Pi model.

Feedback goes to the paired session as a user message. If Pi is busy, it follows the current task. Pi may save feedback in its session history or send it to its model provider. You cannot edit the draft while it waits for a receipt. A receipt confirms only that the extension handed feedback to Pi. If Pi rejects the feedback or the connection drops before the receipt, the draft becomes editable again. After a dropped connection, check Pi before resending.

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
