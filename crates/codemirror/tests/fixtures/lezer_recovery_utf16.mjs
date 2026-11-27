// From the repository root:
// node crates/codemirror/tests/fixtures/lezer_recovery_utf16.mjs <artifact-dir> > crates/codemirror/tests/fixtures/lezer_recovery_utf16.jsonl
import assert from 'node:assert/strict';
import {execFileSync} from 'node:child_process';
import {createHash} from 'node:crypto';
import {readFileSync, realpathSync} from 'node:fs';
import {findPackageJSON} from 'node:module';
import {join, resolve} from 'node:path';
import {pathToFileURL} from 'node:url';

const root = resolve(process.argv[2]);
const inventory = JSON.parse(readFileSync(new URL('../../../../docs/codemirror-upstream.json', import.meta.url)));
const entries = new Map();
for (const name of ['@lezer/json', '@lezer/lr', '@lezer/common']) {
    const pin = inventory.referenceDependencies.find(item => item.name === name);
    const dir = join(root, name.replace('/', '__'));
    const archive = join(dir, pin.tarball.split('/').at(-1));
    const integrity = `sha512-${createHash('sha512').update(readFileSync(archive)).digest('base64')}`;
    assert.equal(integrity, pin.integrity, name);
    const pkg = join(dir, 'package/package.json');
    assert.equal(JSON.parse(readFileSync(pkg)).version, pin.version, name);
    for (const file of ['package/package.json', 'package/dist/index.js']) {
        assert.deepEqual(readFileSync(join(dir, file)), execFileSync('tar', ['-xOf', archive, file]));
    }
    entries.set(name, {pkg: realpathSync(pkg), url: pathToFileURL(join(dir, 'package/dist/index.js'))});
}
for (const [name, dep] of [['@lezer/json', '@lezer/lr'], ['@lezer/lr', '@lezer/common']]) {
    assert.equal(realpathSync(findPackageJSON(dep, entries.get(name).url)), entries.get(dep).pkg);
}
const {parser} = await import(entries.get('@lezer/json').url);

for (const source of [
    '?,,]',            // One deletion pass for one UTF-16 unit.
    '🐍,,]',           // The second pass expires the recovery window earlier.
    '🐍{""🐍@e0\n',    // A branch is already ahead during the second deletion.
    '0{+1:1+1""🐍:',   // Do not spend both passes at the same frontier.
    '🐍',              // The scalar is at the end of the input.
    'true "🐍" ]',     // A recognized token can contain an intact scalar.
]) {
    const byteAt = pos => {
        assert.ok(!/[\uDC00-\uDFFF]/.test(source[pos] ?? ''), `split scalar at ${pos}`);
        return Buffer.byteLength(source.slice(0, pos));
    };
    const nodes = [];
    let depth = 0;
    parser.parse(source).iterate({
        enter: node => nodes.push([depth++, node.name, byteAt(node.from), byteAt(node.to)]),
        leave: () => { depth--; },
    });
    console.log(JSON.stringify({source, nodes}));
}
