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

