// node html_close_tag_recovery.mjs <pinned-archive-directory>
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {pinnedModules} from './pinned_modules.mjs';

const scratch = realpathSync(mkdtempSync(join(tmpdir(), 'html-close-tag-recovery-')));
try {
    const {load} = pinnedModules(resolve(process.argv[2]), scratch);
    const {html} = await load('@codemirror/lang-html');
    const {IterMode, TreeFragment} = await load('@lezer/common');

    function rows(tree, source) {
        const bytes = [0];
        let unit = 0, byte = 0;
        for (const ch of source) {
            unit += ch.length;
            byte += Buffer.byteLength(ch);
            bytes[unit] = byte;
        }
        const result = [];
        const cursor = tree.cursor(IterMode.IgnoreMounts);
        function visit(depth) {
            result.push([depth, cursor.name, bytes[cursor.from], bytes[cursor.to]]);
            if (cursor.firstChild()) {
                do visit(depth + 1); while (cursor.nextSibling());
                cursor.parent();
            }
        }
        visit(0);
        return result;
    }

    for (const matchClosingTags of [true, false]) {
        const parser = html({matchClosingTags}).language.parser;
        for (const [left, right, insert] of [
            ['<a></a', '>tail', ' x'],
            ['<a></A\t', ' />tail', '/'],
            ['<x></a', '>tail', ' ="x"'],
            ['<ol><li>x</ol', '>tail', ' x'],
            ['<名>é\r\n</名\r\n', '><b>後</b>', 'あ🍵'],
            ['<a></a', '><!-- gap --><script>x = 1;</script>', ' x'],
        ]) {
            const source = left + right;
            const broken = left + insert + right;
            const at = left.length, end = at + insert.length;
            const start = parser.parse(source);
            const fragments = (tree, toA, toB) => TreeFragment.applyChanges(
                TreeFragment.addTree(tree), [{fromA: at, toA, fromB: at, toB}]);
            const changed = parser.parse(broken, fragments(start, at, end));
            const repaired = parser.parse(source, fragments(changed, end, at));
            const initial = rows(start, source), recovered = rows(changed, broken);
            assert.deepEqual(recovered, rows(parser.parse(broken), broken));
            assert.deepEqual(initial, rows(repaired, source));
            console.log(JSON.stringify({
                matchClosingTags, source, broken, at: Buffer.byteLength(left), insert,
                initial, recovered,
            }));
        }
        for (const source of ['</a 🍵>tail', '<a></a x<next>tail', '<a></a x', '<a></a \t>tail']) {
            console.log(JSON.stringify({matchClosingTags, source, initial: rows(parser.parse(source), source)}));
        }
    }
} finally {
    rmSync(scratch, {recursive: true, force: true});
}
