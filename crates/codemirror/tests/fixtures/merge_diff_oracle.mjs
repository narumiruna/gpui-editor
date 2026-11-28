// node merge_diff_oracle.mjs <archive-dir> [--audit]
// The default emits the regression fixture. --audit emits all 11,328 audit pairs.
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {pinnedModules} from './pinned_modules.mjs';

function* regressionCases() {
    for (const [name, a, b] of [
        ['weighted-swap', '😀a', 'a😀'],
        ['split-at-final-surrogate', 'a😀', '😀😁'],
        ['match-shared-high-surrogate', 'a😀', '😁a'],
        ['match-shared-low-surrogate', 'a😀', '🨀a'],
        ['match-distinct-surrogates', 'a😀', '𐐀a'],
        ['unequal-unit-lengths', 'aa', '😀a𐐀'],
        ['quarter-seed', 'a😀a', 'aa😀'],
        ['astral-word-expansion', 'a𐐀a', '𐐀😀a'],
        ['presentable-retains-astral-gap', 'a😀a', '😀a😀'],
        ['trim-before-search', 'é\r\naa😀\r\nβ', 'é\r\na😀😁\r\nβ'],
        ['combining-and-line-breaks', '😁\ń\n🨀\r\ŕ', '\n\r\ń😁🨀𐐀😁\n🨀\r\r😁 \n'],
        ['containment', '𐐀😀', 'a𐐀😀β'],
        ['bmp-control', 'abc αβα def', 'abc αζα def'],
        ['absolute-prefix-offset', 'bc\n', '\n\nbc'],
        ['empty-split-retained', 'bbac', 'aca'],
        ['empty-split-joins-display-gaps', 'bb😀b', '😀b\n'],
        ['utf16-prefix-offset', '😀b😀😀😀b', '😀a😀😀'],
        ['empty-split-outside-display-gap', 'abbcba\nba cc\n\ra\nb cbb0', 'abbcbaa\nba a\nb cbb'],
    ]) {
        yield [name, a, b];
        yield [`${name}-reverse`, b, a];
    }
    for (const [name, a, b, scanLimit] of [
        ['zero-scan-interior-line-feed', '𐐀\n𐐀', '😁\nA😀😀', 0],
        ['zero-scan-unequal-unit-seeds', '\nAA😁\r  ', '\r\n𐐀β', 0],
        ['limited-quarter-seed', '\r😁𐐀 😁 β ', 'Aβ\nA', 12],
        ['limited-frontier', 'β😁A\nβ\n', '😀\n\nAA😀', 12],
        ['limited-shared-anchor', 'βA𐐀𐐀😀β𐐀A\r\n😁😁β\r β😁\r\r\r', 'βA𐐀β\n𐐀😀β𐐀A\r\nAββ  A\r𐐀\r \nA\r', 12],
    ]) {
        yield [name, a, b, scanLimit];
        yield [`${name}-reverse`, b, a, scanLimit];
    }
    yield* sizeLimitCases();
}

function* sizeLimitCases() {
    const prefix = 'start é\r\n', suffix = '\r\nend β';
    // The markers make the edits small but block every quarter seed.
    const bmp = 'β'.repeat(35);
    const blocked = (middle, marker) => [prefix, middle, bmp, bmp, bmp.slice(1), suffix].join(marker);
    // Depth 9: the trimmed regions have 144 or 145 UTF-16 units.
    for (const [name, middle, nativePrecise] of [
        ['bmp-at-myers-size-limit', bmp, true],
        ['bmp-over-myers-size-limit', bmp + 'β', false],
        ['astral-over-myers-size-limit', 'β'.repeat(16) + '😀' + 'β'.repeat(18), false],
    ]) {
        yield [name, blocked(middle, 'X'), blocked(middle, 'Y'), 18, nativePrecise];
    }

    // Depth 5: the smaller region has 320 or 321 units. The shared run is
    // a coarse seed, but is too short to be a quarter seed.
    const coarse = (pad, end) => prefix + pad.repeat(53) + 'k'.repeat(53) + pad.repeat(213) + end + suffix;
    const b = coarse('ζ', 'ζζ');
    yield ['bmp-at-replacement-size-limit', coarse('α', 'α'), b, 10, false];
    yield ['astral-over-replacement-size-limit', coarse('α', '😀'), b, 10, false];

    // The unmatched region exceeds both size limits, but a quarter seed
    // isolates two one-character replacements without a fallback.
    const keep = Array.from({length: 32}, (_, i) => `K${String(i).padStart(3, '0')}\n`).join('');
    yield ['quarter-anchor-before-size-limit', `A${keep}B`, `C${keep}D`, 4, true];
}

function words(alphabet) {
    const result = [''];
    let level = [''];
    for (let size = 1; size <= 3; size++) {
        level = level.flatMap(word => alphabet.map(scalar => word + scalar));
        result.push(...level);
    }
    return result;
}

function* auditCases() {
    for (const [name, alphabet] of [
        ['ascii-exhaustive', ['a', 'b', ' ']],
        ['bmp-exhaustive', ['a', 'α', '\u0301']],
        ['high-exhaustive', ['a', '😀', '😁']],
        ['low-exhaustive', ['a', '😀', '🨀']],
        ['distinct-exhaustive', ['a', '😀', '𐐀']],
    ]) {
        const inputs = words(alphabet);
        for (const a of inputs) for (const b of inputs) yield [name, a, b];
    }

    const ascii = ['a', 'b', 'c', '0', ' ', '\r', '\n'];
    const bmp = ['a', 'é', 'β', '²', '\u0301', ' ', '\r', '\n'];
    const astral = ['a', 'é', 'β', '😀', '😁', '🨀', '𐐀', '\u0301', ' ', '\r', '\n'];
    let state = 17;
    function next(n) {
        state = (Math.imul(state, 1_664_525) + 1_013_904_223) >>> 0;
        return (state >>> 16) % n;
    }
    function text(alphabet, size) {
        return Array.from({length: size}, () => alphabet[next(alphabet.length)]).join('');
    }
    for (const [name, alphabet] of [['ascii-random', ascii], ['bmp-random', bmp], ['astral-random', astral]]) {
        for (let i = 0; i < 768; i++) yield [name, text(alphabet, next(29)), text(alphabet, next(29))];
    }
    for (const [name, alphabet] of [['ascii-edits', ascii], ['astral-edits', astral]]) {
        for (let i = 0; i < 512; i++) {
            const a = text(alphabet, next(28) + 6), b = [...a];
            for (let j = 0; j < 3; j++) b.splice(next(b.length + 1), next(5), ...text(alphabet, next(5)));
            yield [name, a, b.join('')];
        }
    }
}

function byteOffsets(text) {
    let units = 0, bytes = 0;
    const offsets = new Map([[0, 0]]);
    for (const scalar of text) offsets.set(units += scalar.length, bytes += Buffer.byteLength(scalar));
    return offsets;
}

function toBytes(a, b, ma, mb, changes) {
    let posA = 0, posB = 0;
    const result = changes.map(({fromA, toA, fromB, toB}) => {
        assert(ma.has(fromA) && ma.has(toA) && mb.has(fromB) && mb.has(toB), 'non-scalar endpoint');
        assert(posA <= fromA && fromA <= toA && posB <= fromB && fromB <= toB, 'unordered range');
        assert.equal(a.slice(posA, fromA), b.slice(posB, fromB), 'unequal gap');
        posA = toA;
        posB = toB;
        return [ma.get(fromA), ma.get(toA), mb.get(fromB), mb.get(toB)];
    });
    assert.equal(a.slice(posA), b.slice(posB), 'unequal suffix');
    return result;
}

const scratch = mkdtempSync(join(realpathSync(tmpdir()), 'gpui-merge-diff.'));
try {
    const {load} = pinnedModules(resolve(process.argv[2]), scratch);
    const {diff, presentableDiff} = await load('@codemirror/merge');
    const cases = process.argv[3] === '--audit' ? auditCases() : regressionCases();
    for (const [name, a, b, scanLimit, nativePrecise] of cases) {
        const options = {scanLimit: scanLimit ?? 500}; // Match native DiffConfig::default().
        const ma = byteOffsets(a), mb = byteOffsets(b);
        const raw = toBytes(a, b, ma, mb, diff(a, b, options));
        const display = toBytes(a, b, ma, mb, presentableDiff(a, b, options));
        console.log(JSON.stringify({name, a, b, scanLimit, raw, display, nativePrecise}));
    }
} finally {
    rmSync(scratch, {recursive: true, force: true});
}
