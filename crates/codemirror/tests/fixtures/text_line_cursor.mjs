// Replay with node text_line_cursor.mjs ARCHIVE_ROOT. Both archives must be local.
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {pinnedModules} from './pinned_modules.mjs';

const archives = resolve(process.argv[2]);
realpathSync(join(archives, '@codemirror__state', 'state-6.7.6.tgz'));
realpathSync(join(archives, '@marijn__find-cluster-break', 'find-cluster-break-1.0.4.tgz'));
const root = mkdtempSync(join(realpathSync(tmpdir()), 'text-line-cursor-'));
try {
    const modules = pinnedModules(archives, root);
    assert.equal(modules.pins.get('@codemirror/state').version, '6.7.6');
    assert.equal(modules.pins.get('@marijn/find-cluster-break').version, '1.0.4');
    const {Text} = await modules.load('@codemirror/state');
    // Numbers are skips in both systems. Pairs give native bytes, then JS code units.
    // Either byte of CRLF maps to the start of one LF in the JS document.
    const cases = [
        ['empty', '', [0, 0, -1, 0]],
        ['only-breaks', '\n\r\n', [0, 0, 0, 0, -10]],
        ['skip-to-break', 'a\nb', [1, 0, 0]],
        ['skip-past-break', 'a\nb', [2, 0, 0]],
        ['skip-to-cr', 'a\r\nb', [1, 0, 0]],
        ['skip-to-lf', 'a\r\nb', [[2, 1], 0, 0]],
        ['skip-past-crlf', 'a\r\nb', [[3, 2], 0, 0]],
        ['mixed-breaks-and-lone-cr', 'α\r\n\r\nβ\rγ\n', [0, 0, 0, 0, 0]],
        ['forward-and-rewind-in-content', 'a💡éβ\nω', [[5, 3], [-2, -1], [-8, -4], 0, 0]],
        ['sticky-done-with-new-upstream-value', 'a💡\né', [1, 0, [-2, -1], 0, [-2, -1], 0, -100, 0]],
        ['rewind-after-final-empty-before-done', 'a\n', [0, 0, -2, 0, 0, -2, 0]],
        ['rewind-into-either-crlf-byte', 'a\r\nb\r\nc', [0, 0, [-2, -2], [-3, -2], [-4, -3], 0, 0, 0]],
        ['clamp-start-then-end-after-content', 'a\r\nb\n', [-100, 100, -100, 0]],
        ['clamp-end-before-content', 'a\r\nb\n', [100, -100, 0, 0, 0, 0, -100]],
        ['skip-to-lf-at-leaf-seam', `${'x'.repeat(1023)}\r\nb`, [[1024, 1023], 0, 0]],
        ['skip-across-tree-branches', `${'é\n'.repeat(345)}tail`,
            [[1035, 690], [-1039, -694], [1033, 689], 0, [-7, -6], 0, 0]],
        ['ranged-middle', 'outside\nabc\ndef\noutside', [0, 2, -4, 0, -100], [2, 4]],
        ['ranged-rewind', 'outside\nabc\ndef\nghi\noutside', [0, 2, -100, 0, 0, 0], [2, 5]],
        ['ranged-after-content', 'outside\nabc\ndef\noutside', [0, 0, -100, 0], [2, 4]],
        ['ranged-single-empty', 'outside\r\n\r\nlast', [0, 0, 1], [2, 3]],
        ['ranged-multiple-empty', 'outside\n\n\nlast', [0, 0, -1, 0], [2, 4]],
        ['ranged-final-line', 'skip\r\nb\r\n', [0, 0, -100, 0], [2, 4]],
        ['ranged-collapsed', 'outside\nb\nc', [0, 0, 1], [2, 2]],
        ['ranged-skip-past-lf', 'skip\r\nabc\r\né💡\nlast', [[5, 4], 0, 0], [2, 4]],
        ['ranged-skip-to-crlf', 'skip\r\nabc\r\né💡\nlast', [[4, 3], 0, 0], [2, 4]],
        ['ranged-skip-past-limit', 'skip\na\nb\nlast', [100, -100, 0], [2, 4]],
    ];
    for (const [id, source, skips, bounds] of cases) {
        const cursor = Text.of(source.split(/\r?\n/u)).iterLines(...(bounds ?? []));
        assert.deepEqual([cursor.value, cursor.lineBreak, cursor.done], ['', false, false]);
        const steps = skips.map(pair => {
            const [bytes, units] = Array.isArray(pair) ? pair : [pair, pair];
            const {value, lineBreak, done} = cursor.next(units);
            assert.ok(value.isWellFormed(), id);
            return [bytes, units, value, lineBreak, done];
        });
        console.log(JSON.stringify({id, source, steps, ...(bounds && {bounds})}));
    }
    // A UTF-16 skip can return only the low half of an astral scalar. Record the
    // numeric code units rather than putting an unpaired surrogate in the fixture.
    const source = 'a💡\né';
    const cursor = Text.of(source.split('\n')).iterLines();
    const split = ({value}) => Array.from({length: value.length}, (_, i) => value.charCodeAt(i));
    const initial = split(cursor.next(2));
    const rewind = split(cursor.next(-1));
    console.log(JSON.stringify({id: 'invalid-scalar-boundary', source, initial, rewind}));
} finally {
    rmSync(root, {recursive: true, force: true});
}
