// Replay with node text_splice.mjs ARCHIVE_ROOT.
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {pinnedModules} from './pinned_modules.mjs';

const root = mkdtempSync(join(realpathSync(tmpdir()), 'text-splice-'));
try {
    const {Text} = await pinnedModules(resolve(process.argv[2]), root).load('@codemirror/state');
    const cases = [
        {name: 'empty pair', source: [''], insert: [''], range: [0, 0]},
        {name: 'whole replacement', source: ['é', 'β'], insert: ['', '🦀', ''], range: [0, 3]},
        {name: 'insert at start', source: ['🙂', 'x'], insert: ['prefix', ''], range: [0, 0]},
        {name: 'append after terminal line break', source: ['🦀', ''], insert: ['é', '尾'], range: [3, 3]},
        {name: 'replace astral and break', source: ['é💡', 'β'], insert: ['🌱', '', 'α'], range: [1, 4]},
        {name: 'delete across empty lines', source: ['top', '', 'bottom'], insert: [''], range: [2, 7]},
        {name: 'insert retained source slice', source: ['à', '😀', '', 'β'], slice: [1, 5], range: [7, 7]},
        {name: 'insert a document into itself', source: ['x', 'α', ''], range: [1, 1]},
        {name: 'cross tree branches', source: Array.from({length: 75}, (_, i) => `${i}🦀`),
            insert: Array.from({length: 40}, (_, i) => `λ${i}`), range: [182, 239]},
    ];
    for (const {name, source, insert, slice, range} of cases) {
        const doc = Text.of(source);
        const raw = doc.toString();
        const at = units => {
            assert.ok(units === raw.length || !/^[\udc00-\udfff]/u.test(raw.slice(units)));
            return Buffer.byteLength(raw.slice(0, units));
        };
        const fragment = slice ? doc.slice(...slice) : insert ? Text.of(insert) : doc;
        const changed = doc.replace(...range, fragment);
        const appended = doc.append(fragment);
        assert.deepEqual(doc.toJSON(), source);
        console.log(JSON.stringify({
            name, source, insert, slice: slice?.map(at), range: range.map(at),
            changed: changed.toJSON(), appended: appended.toJSON(),
            equal: [changed.eq(Text.of(changed.toJSON())), changed.eq(doc), appended.eq(changed)],
        }));
    }
} finally {
    rmSync(root, {recursive: true, force: true});
}
