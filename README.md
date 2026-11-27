# GPUI Editor

GPUI Editor is a local editor for Markdown and UTF-8 plain text on macOS and Windows. It uses GPUI to show formatted text, tables, code blocks, images, links, Mermaid flowcharts, and LaTeX formulas in one editing surface. It saves Markdown without rewriting parts you did not edit.

## Build and run on macOS

Install Rust and Xcode. The native build was verified on Apple Silicon with macOS 26.4.1, Rust 1.95.0, and Xcode 26.6. Other configurations are not yet verified. GPUI compiles shaders at runtime, so the separate Metal compiler component is not required.

Install [Just](https://just.systems/) and run this from the repository root:

```sh
just run
```

This builds and starts the app. It uses `/Applications/Xcode.app` by default. If Xcode is installed elsewhere, set `DEVELOPER_DIR` to that installation's `Contents/Developer` directory before running `just run`.

## Build and run on Windows

Install Rust with the MSVC toolchain and Visual Studio Build Tools with **Desktop development with C++**, including a Windows SDK. From the repository root, run:

```sh
cargo run --locked
```

You can also use `just run` if Just is installed. The app uses a native Windows title bar. Pi pairing works on Windows, but window capture does not.

## Use the editor

The shortcut examples below use macOS key names. On Windows, use **Ctrl** for **Cmd** and **Alt** for **Option**. To open Preferences on Windows, press `Ctrl-,`.

The command takes no path. If there is no workspace to restore, choose **Open Folder…** in the sidebar. Expand folders in the file tree to browse their contents. The tree shows ordinary files and folders without following symlinks. Click a text file in the tree or follow a Markdown link within the workspace to open a tab or select its existing tab. **New note** opens a separate blank Markdown tab.

Drag the divider to resize the sidebar. Use the sidebar chevron, **View → Toggle Sidebar**, or the default `Cmd-/` shortcut to hide or show it without changing its state or the active document tab. When the sidebar is hidden, **Open Folder…** and **New note** move to the editor toolbar. The macOS window controls share the top row; drag an empty part of that row to move the window.

### Change keyboard shortcuts

Open **Preferences… → Hotkeys**. Select a shortcut, then press **Cmd** with a letter, number, or punctuation key. You can also hold **Option**, **Control**, or **Shift**. Press `Esc` to cancel recording. A shortcut cannot use a reserved combination or one assigned to another command.

Changes take effect after the editor saves them. The editor keeps them across restarts in `~/Library/Application Support/GPUI Editor/hotkeys.json` on macOS or `%LOCALAPPDATA%\GPUI Editor\hotkeys.json` on Windows. Use **Reset Defaults** to restore all six command shortcuts. The shortcuts for opening Preferences and quitting the editor, and the standard text-editing shortcuts, do not change.

### Run commands from the keyboard

By default, press `Cmd-P` to open the Command Palette. Type part of a command name to find **Open Folder…**, **New note**, **Save**, or **Toggle Sidebar**, regardless of letter case. Use the up and down arrow keys to select a command and `Enter` to run it. If you use an input method, finish composing before running the command. Press `Esc` to close the palette without running a command and return to your previous focus.

The palette uses the same rules as the toolbar and menus. For example, you cannot run **Save** without a document, and opening a different folder still checks for unsaved changes. To find files or text in the workspace, use **Search** in the sidebar instead.

### Find documents in the sidebar

The sidebar starts on **Tree**. Select a tab below the sidebar toolbar to show one view at a time. At narrow widths, hover over a tab icon to see its name. Drag a sidebar tab to reorder it. Switching or reordering sidebar tabs does not switch, save, or reorder document tabs.

After clicking a sidebar tab, use `Tab` or `Shift-Tab` to move between sidebar controls, `Enter` or `Space` to activate a focused button, and `Esc` to return to the document.

- **Tree:** Expand folders and click a file to open it. Hover over a file row or give it keyboard focus to pin or unpin the file without opening it.
- **Recent:** Reopen one of the 50 most recently opened files. Opening a file again moves it to the top without adding a duplicate. Clicking an already open document tab, restoring a session, and saving a new file for the first time do not count as opens.
- **Pins:** Reopen pinned files even after closing their document tabs. Hover over a row or give it keyboard focus to remove its pin without closing or deleting the file. **Recent** and **Pins** show workspace-relative paths to distinguish files with the same name.
- **Outline:** Click a heading to scroll to it in the current Markdown document, including an unsaved or untitled draft. This does not move the caret or change the selection.
- **Search:** Type a single line and press `Enter` after finishing any input method composition. Search matches the exact text, including letter case, in UTF-8 `.md` filenames and contents saved on disk, not in drafts. Click a filename match to open the file, or a content match to open it and scroll to the match. A stale content match leaves your draft intact and asks you to search again instead of guessing where to jump. Search does not replace text.

The selected sidebar view, tab order, and width stay in the window when you hide the sidebar or switch workspaces. Recent files, pins, and the search query and results stay until you successfully switch to a different workspace. None of these sidebar settings or lists return on restart. Document tabs still restore as described below.

### Work with tabs

- Click a tab to switch without saving. Each open tab keeps its own draft, caret, and undo history. The tab bar shows the selected tab and marks unsaved changes. Scroll the tab bar if not all tabs fit. **Save** or `Cmd-S` saves only the selected tab.
- Drag a document tab to reorder it without switching documents or changing drafts, carets, or undo history. The insertion line shows where the tab will go. Drag near either edge to scroll to tabs that do not fit. Drop outside the document tab row or press `Esc` to cancel. Sidebar tabs and document tabs cannot move between rows.
- Use a tab's close button or `Cmd-W` to close a tab. `Cmd-W` targets the selected tab. When you close the selected tab, the editor selects the tab to its right, or the one to its left if there is none on the right. Closing the last tab leaves the workspace and sidebar open. With no tabs, `Cmd-W` closes the window.
- Closing a tab checks that tab for unsaved changes. Switching to a different workspace, using **Close Window** or the window close button, or quitting with `Cmd-Q` checks all tabs in order. For each draft, choose **Save**, **Don’t Save**, or **Cancel**. If you cancel or a save fails, the editor returns to the original tab. Changes already saved stay saved; the other drafts stay open. Choosing the same workspace again only refreshes the sidebar.
- Finish or cancel any active input method composition before saving, closing, or switching tabs. Then try the action again.

### Reopen the last workspace

After a normal exit, the next launch opens the last workspace and its saved tabs in their last order, including any changes made by dragging. It skips missing or unreadable files and selects the last selected tab if it can be restored, or the first available tab. If no tabs can be restored, it opens only the workspace. If the workspace is missing, inaccessible, or has been replaced, choose a folder again with **Open Folder…**. A missing or unreadable session record does not prevent manual selection, and the editor never opens the Terminal's current folder automatically.

The editor stores the folder and tab list locally in `~/Library/Application Support/GPUI Editor/session.json` on macOS or `%LOCALAPPDATA%\GPUI Editor\session.json` on Windows, not in the workspace. It does not autosave documents or recover drafts after a crash. Standalone files saved outside a workspace, caret positions, undo history, remote image permissions, Developer Mode, and Pi pairings do not return on restart. If the editor cannot update the session record when closing, it leaves the previous record in place and offers **Keep Window Open** or **Quit Without Updating**.

### Edit plain text

Open existing UTF-8 files such as `.txt`, `.json`, source files, or files without an extension from **Tree**. Files other than `.md` appear as plain text in a monospaced font. They do not use Markdown tools, previews, or code fence completion. Save keeps the filename, the extension, and the line endings you did not edit.

Source and configuration files use syntax colors based on their extension, including `.py`, `.rs`, `.go`, `.ts`, `.js`, `.toml`, `.yaml`, `.yml`, and `.json`. The editor does not run the code or reformat it. Unrecognized extensions, files without an extension, and files with a line longer than 16 KiB keep the default text color.

Plain text files are limited to 1 MiB. Files with invalid UTF-8 or control characters other than tab, carriage return, and line feed do not open as plain text.

Undo reverses a run of typing in one step. A pause of one second or more, moving the caret, or changing the selection ends the run. Paste, line breaks, deletion, formatting, and each input method composition have their own undo steps.

### Edit Markdown

- Press `Cmd-F` to find text in the current draft, including unsaved edits. The editor marks matches in the visible text. Use `Enter` or `Shift-Enter` in the Find field, or `Cmd-G` or `Cmd-Shift-G` in the editor, to select the next or previous match. `Esc` in the panel closes it and returns focus to the editor.
- Press `Cmd-D` to add the next occurrence or `Cmd-Shift-L` to select all occurrences of the selected text. In Find, choose **Select all matches** to select matches of the query. Edits apply to every selection in one undo step or make no change if a selection is read-only. Press `Esc` to keep only the primary selection.
- Press `Cmd-Option-G` to go to a line. Enter an absolute line, a relative offset such as `+5`, or a percentage. Add a zero-based column after `:`.
- Use **Replace…** in the Find panel to replace the selected match or all matches. **Replace All** is one undo step. If any match cannot be edited under the Markdown rules, **Replace All** does not change the draft. Use the sidebar **Search** to search other saved files.

- Use the toolbar below the tabs to save, format a selection, insert a diagram, add an inline formula, or insert a link. Hover over an icon to see its name. At narrow window widths, scroll the toolbar to reach more actions. The three-dot **More tools** button shows front matter, display formulas, images, and item actions. Click front matter, a diagram, or a formula to edit its source. Click elsewhere in the document to preview the change; this does not save the file.
- Source editing shows a type label such as `<mermaid>` and a border around the editable text. The label and border are not part of the Markdown.
- Inline code uses a monospaced font. Click inside it or move the caret into it to show its backticks and edit the Markdown. The backticks hide again when the caret leaves. The editor keeps them in the saved file.
- Type three backticks at the start of an empty line to insert a matching closing fence and put the caret on the blank line between them. The block stays editable until you move the caret outside it. Pasting Markdown does not insert a closing fence.
- In an editable code fence with native syntax folds, press `Ctrl-Shift-[` or choose **More tools** → **Toggle code fold**. Click the inline ellipsis to expand it. Folding does not change the draft.
- Use `Ctrl-Space` for completion in supported code fences. Go, Python, JavaScript, and TypeScript offer snippets. After inserting a snippet, use `Tab` or `Shift-Tab` to move between fields and `Esc` to end the session. In an editable fence with native syntax, use `Cmd-I` to select the next parent syntax node. In editable code fences, `Option-Up` and `Option-Down` move lines. Add `Shift` to duplicate instead. In supported fences, `Enter` inserts an indented line, `Cmd-Enter` adds one below the current line, and `Cmd-Option-\` indents the selected lines. Use `Cmd-/` to toggle the language's comments.
- Paste a URL over selected editable prose to make a link. In editable bullet lists, `Enter` continues the list and `Backspace` after a marker removes it.
- On a Markdown tab with unsaved edits, choose **More tools** → **Review changes** to compare the draft with its saved text. Use **Restore saved change** to undo one group of changes without saving.
- For links, hover to see the full destination in a tooltip. Click to open the web browser or follow an in-workspace `.md` link. To edit a link without opening it, place the caret at its edge and choose **More tools** → **Edit link**.
- Image previews support local or HTTPS PNG, JPEG, GIF, and WebP still images. Relative paths are based on the document's folder. To fetch HTTPS previews, choose **Load remote images for this document** in the editor toolbar. The remote host receives the image request; redirects are not followed.
- GFM tables show a grid with a header and aligned plain-text cells. Closed fenced blocks and indented code blocks show monospaced text. Supported fenced languages show syntax colors. In dark appearance, the Markdown editor uses One Dark. Table and code previews are read-only and scroll with the document. An unclosed code or Mermaid fence stays editable as Markdown until you add its closing fence. Prose wraps to fit the editor; wide previews can scroll horizontally. Click either preview to select the entire block; `Cmd-C` copies its original Markdown.
- The editor supports a subset of Markdown. Inline math cannot span paragraph breaks. The Mermaid renderer supports flat flowcharts; unsupported blocks such as block quotes are shown as read-only source.
- The editor stops a save if it detects an earlier external change. Existing files need a filesystem that supports atomic exchange. If a competing version appears during the exchange, the app reports a conflict and gives the path to the displaced version in a private `.gpui-save-*` folder. The draft may already be at the original path. Review both versions before continuing. If the editor reports that the folder moved during a first save, the draft may be in the moved folder. Check that folder before retrying. If a different folder takes the workspace path, select it again to switch workspaces.

LaTeX previews use STIX Two Math through Mathra. See the [STIX Two Math license](docs/licenses/stix-two-math.txt).

## Develop

The Cargo workspace separates the [app](crates/app/), [editor core](crates/codemirror/), [Markdown model](crates/document/), [previews](crates/preview/), and [file handling](crates/workspace/). App-only changes reuse the other compiled crates. The crates share the root `target/` directory and lockfile.

Run checks for a single component with `-p`. For example, this tests the Markdown model without building GPUI:

```sh
cargo test --locked -p gpui-editor-document
```

Syntax definitions come from [Two Face](https://docs.rs/two-face/) and its [embedded grammar acknowledgements](https://codeberg.org/CosmicHarper/two-face/src/tag/v0.5.2+bat-0.26.1/generated/acknowledgements_full.md).

The app and its internal crates build from source as a workspace. They are not published separately to crates.io.

### Developer Mode and Pi

Pi pairing works on macOS and Windows. Run GPUI Editor and Pi on the same computer under the same OS account. On Windows, run Pi in Windows, not in WSL. Window capture is available only on macOS.

1. Start GPUI Editor. In **Preferences…** (`Cmd-,` on macOS or `Ctrl-,` on Windows), turn on **Developer Mode**. A **Select element** button with a target icon appears in the upper-right corner.
2. Start Pi 1.6.0 or later in this repository. Trust the project if you want Pi to load [its local extension](.pi/extensions/gpui-editor.ts). In Pi, run `/gpui-pair`. If Pi finds more than one editor, choose the one to pair. Verify the code in both apps, then choose **Pair** in GPUI Editor.
3. Click **Select element**, then click an element such as **Save**, a sidebar row, or the editing surface. Picking does not run the element's usual action. Press `Esc` to leave picking mode.
4. Write your feedback. On macOS, choose **Capture Window** to attach a screenshot of GPUI Editor. On either platform, choose **Paste Image** to attach a PNG, JPEG, or TIFF image from the clipboard, or send text only. Review attachments before choosing **Send to Pi**. An image requires an image-capable Pi model.

Feedback goes to the paired session as a user message. If Pi is busy, it follows the current task. Pi may save feedback in its session history or send it to its model provider. You cannot edit the draft while it waits for a receipt. A receipt confirms only that the extension handed feedback to Pi. If Pi rejects the feedback or the connection drops before the receipt, the draft becomes editable again. After a dropped connection, check Pi before resending.

With a live pairing, Pi can call `gpui_editor_logs` to read a bounded history of editor diagnostics. It does not read documents, raw paths, or system logs. To disconnect, use `/gpui-unpair` in Pi, choose **Unpair Pi** in Preferences, or turn Developer Mode off. Pi session changes, reloads, and exits also disconnect. The editor starts with the mode off and does not reopen pairings. See [ADR 0002](docs/decisions/0002-local-pi-session-pairing.md) for the data boundaries.

## Checks

Install [Bun](https://bun.com/) to run the Pi extension tests on macOS or Windows.

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --all-targets
prek run --all-files
```

On macOS and Windows, also run:

```sh
bun test ./.pi/tests/gpui-editor.test.ts
```

GitHub Actions runs Rust checks with Rust 1.95.0 on Linux, macOS, and Windows. The Linux job checks the portable document, rendering, and file-handling code. The macOS and Windows jobs also build the GPUI application. Automated GPUI tests use a simulated window and do not verify native input, drag behavior, or dialogs. If macOS asks the editor to close during logout or restart, the editor can protect unsaved changes, but the system request must be started again after the editor closes. See the [manual acceptance steps](docs/prd/gpui-editor.md#macos-原生驗收) for the remaining native checks.

[Changenote](https://github.com/changenote/changenote) uses [`.changenote/`](.changenote/) to maintain release notes and a release pull request. The [release workflow](.github/workflows/release.yml) pins both the Action and CLI versions. It uses the optional `PAT_TOKEN` repository secret for Changenote and falls back to the built-in `GITHUB_TOKEN` if the secret is not set. Use a fine-grained personal access token with write access to Contents and Pull requests for this repository if you need release pull requests to trigger CI without manual approval. After a verified release pull request merges, the workflow pushes Git tags and creates GitHub Releases. Changes to the editor do not require a manual release workflow run.
