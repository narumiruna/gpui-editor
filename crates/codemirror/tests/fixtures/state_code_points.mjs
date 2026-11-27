// node crates/codemirror/tests/fixtures/state_code_points.mjs <archive-root>
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {pinnedModules} from './pinned_modules.mjs';

const root = realpathSync(mkdtempSync(join(tmpdir(), 'state-code-points-')));
try {
    const {load} = pinnedModules(resolve(process.argv[2]), root);
    const {codePointAt, codePointSize, fromCodePoint} = await load('@codemirror/state');
    const codePoints = [0, 0x41, 0xd7ff, 0xd800, 0xdbff, 0xdc00, 0xdfff, 0xe000, 0xffff, 0x10000, 0x1f600, 0x10ffff];
    const strings = ['', 'é😀猫', '\ud800A\udfff', '\ud800\ud800\udc00\udc00', '\udbff\udfff\ud800'];
    const units = str => Array.from({length: str.length}, (_, i) => str.charCodeAt(i));
    console.log(JSON.stringify({
        encode: codePoints.map(point => [point, codePointSize(point), units(fromCodePoint(point))]),
        decode: strings.map(str => [units(str), Array.from({length: str.length + 1}, (_, i) => codePointAt(str, i))]),
    }));
} finally {
    rmSync(root, {recursive: true, force: true});
}
