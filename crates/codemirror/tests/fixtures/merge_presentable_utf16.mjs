// Replay with node and the directory of SHA-512-pinned archives.
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {join, resolve} from 'node:path';
import {tmpdir} from 'node:os';
import {pinnedModules} from './pinned_modules.mjs';

const cases = [
    ['abcd', 'ab😀😀cd'],
    ['abcd', 'ab😀x😀cd'],
    ['abcd', 'ab😀xcd'],
    ['abcd', 'ab😀cd'],
    ['abcd', 'ab😀\r\ncd'],
    ['hello abcd world', 'hello ab😀😀cd world'],
    ['A😀B', 'C😀D'],
    ['A😀.B', 'C😀.D'],
    ['A.😀B', 'C.😀D'],
    ['A😀😀B', 'C😀😀D'],
    ['A.B', 'C.D'],
    ['A..B', 'C..D'],
    ['A...B', 'C...D'],
    ['éA😀.B\r\nx', 'éC😀.D\r\nx'],
];
const scratch = mkdtempSync(join(realpathSync(tmpdir()), 'gpui-merge-utf16.'));
try {
    const {load} = pinnedModules(resolve(process.argv[2]), scratch);
    const {diff, presentableDiff} = await load('@codemirror/merge');
    const byte = (source, index) => {
        const previous = source.charCodeAt(index - 1);
        const next = source.charCodeAt(index);
        assert(!(previous >= 0xd800 && previous <= 0xdbff && next >= 0xdc00 && next <= 0xdfff));
        return Buffer.byteLength(source.slice(0, index));
    };
    for (const pair of cases) {
        for (const [a, b] of [pair, [...pair].reverse()]) {
            const convert = changes => changes.map(({fromA, toA, fromB, toB}) => [
                byte(a, fromA), byte(a, toA), byte(b, fromB), byte(b, toB),
            ]);
            const raw = convert(diff(a, b));
            const display = convert(presentableDiff(a, b));
            console.log(JSON.stringify({a, b, raw, display}));
        }
    }
} finally {
    rmSync(scratch, {recursive: true, force: true});
}
