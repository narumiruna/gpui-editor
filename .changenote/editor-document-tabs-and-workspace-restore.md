---
gpui-editor: minor
$type: feat
---

Open multiple Markdown documents in tabs in one workspace. Switch tabs without saving and keep each tab’s draft, caret, and undo history. `Cmd-S` saves the selected tab, and `Cmd-W` closes it. Switching workspaces or quitting checks all tabs for unsaved changes. After a normal exit, reopen the last workspace and its saved tabs in order, with the last selected saved tab active when available. Drafts are not saved for restart.
