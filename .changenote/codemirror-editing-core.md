---
gpui-editor: patch
$type: fix
---

Keep each input method composition in one undo step. Undo and redo now store changed text instead of full Markdown snapshots.
