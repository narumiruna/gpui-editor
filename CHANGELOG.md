## 0.4.0

### Minor Changes

- Add opt-in line numbers and lint gutters in standalone files and fold gutters in supported JSON and JavaScript-family files. Use configured language rules for more folding, indentation, and bracket matching in Markdown code drafts. Improve native completion, search, and syntax recovery while preserving original source bytes.

## 0.3.0

### Minor Changes

- Use configured JSON and Lezer delimiter roles and fold rules in editable Markdown code fences.

- Display Markdown inline code in a monospaced font. Show its backticks when the caret enters the span so you can edit the source in place.

- Add Find and Replace, multiple selections, and Go to Line. In supported Markdown code fences, add native syntax services such as highlighting, folding, and completion. Review and restore individual groups of unsaved changes without saving the document.

- Open and edit existing UTF-8 plain text files from the workspace tree, including source files and files without an extension. Markdown files keep their formatted view. Plain text uses a monospaced font and keeps its filename and untouched line endings when saved.

- Highlight syntax in editable Python, Rust, Go, TypeScript, JavaScript, TOML, YAML, JSON, and other recognized source and configuration files. Unrecognized formats still open as plain text.

- Pair GPUI Editor with Pi on Windows to send feedback and read recent editor diagnostics. Windows supports pasted images; native window capture remains available only on macOS.

- Run the Markdown editor on Windows instead of exiting with a macOS-only error. Support native Windows shortcuts, settings, session restore, image previews, and conflict-checked file saves. Window capture remains macOS-only.

### Patch Changes

- Keep each input method composition in one undo step. Undo and redo now store changed text instead of full Markdown snapshots.

- Replace only the primary Find match, paste copied CRLF lines above target carets, block deletion keys in read-only editors, and use displayed line boundaries for rectangular selections in code fences.

- Undo a run of typing in one step. Moving the caret, changing the selection, or pausing for at least one second ends the run. Paste, line breaks, and input method composition stay separate.

## 0.2.0

### Minor Changes

- Add a Hotkeys tab in Preferences to change app shortcuts, keep them across restarts, and restore defaults.

### Patch Changes

- Use Cmd-/ to hide or show the sidebar instead of Cmd-Option-S.

- Complete a new backtick fence and place the caret on the editable blank line inside it.

- Show a type label and border while editing the source of diagrams, formulas, front matter, images, and links.

- Show the pin and unpin icons in the Tree and Pins sidebar views.

- Restore the icons for all five sidebar tabs.

- Keep code and Mermaid source editable until a matching closing fence is entered.

## 0.1.0

### Minor Changes

- Add Developer Mode for local pairing with Pi. Pick an editor element and send feedback with text or a screenshot to the paired session. The session can read recent editor diagnostics while the pairing is active. The feedback field supports the Edit menu and locks the draft until Pi replies or disconnects.

- Give the Markdown editor a full-height file tree and an editor pane with tabs above the formatting tools. Browse nested folders, resize or hide the sidebar, and edit wrapped prose without a separate title bar or status bar.

- Open multiple Markdown documents in tabs in one workspace. Switch tabs without saving and keep each tab’s draft, caret, and undo history. `Cmd-S` saves the selected tab, and `Cmd-W` closes it. Switching workspaces or quitting checks all tabs for unsaved changes. After a normal exit, reopen the last workspace and its saved tabs in order, with the last selected saved tab active when available. Drafts are not saved for restart.

- Add a macOS Markdown editor with a local workspace, source-preserving saves, and hover labels for the main toolbar icons and a single Markdown editing surface. Keep the caret visible during edits, preserve closing heading markers during keyboard navigation, preserve competing file versions on save conflicts, and ask before loading remote images. Do not save provisional input method text or report a save as successful when the destination cannot be verified. Render application text with native macOS fonts.

- Render GFM tables as aligned grids and ordinary code blocks as monospaced text in the editor. Both previews are read-only. Click a block to select it and copy its original Markdown. Mermaid diagrams still render and support in-place source editing.

- Press `Cmd-P` to find and run Open Folder, New note, Save, or Toggle Sidebar from the Command Palette. Drag document tabs to reorder them without switching documents or changing drafts. After a normal exit, saved workspace tabs reopen in their new order.

- Find Markdown documents from five sidebar tabs: Tree, Recent, Pins, Outline, and Search. Reopen recently opened or pinned files, jump to headings in the current draft, and search saved Markdown filenames and contents in the workspace. Drag sidebar tabs to reorder them without changing document tabs. Sidebar state is not restored on restart.

