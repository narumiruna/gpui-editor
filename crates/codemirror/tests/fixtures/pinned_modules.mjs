import assert from 'node:assert/strict';
import {execFileSync} from 'node:child_process';
import {createHash} from 'node:crypto';
import {existsSync, mkdirSync, readFileSync, realpathSync} from 'node:fs';
import {findPackageJSON} from 'node:module';
import {join} from 'node:path';
import {pathToFileURL} from 'node:url';

const manifest = JSON.parse(readFileSync(new URL('../../../../docs/codemirror-upstream.json', import.meta.url)));
const pins = new Map([...manifest.packages, ...manifest.referenceDependencies, ...manifest.runtimeDependencies]
    .map(pin => [pin.name, pin]));

function importEntry(target) {
    if (typeof target === 'string') return target;
    for (const [condition, next] of Object.entries(target ?? {})) {
        if (['node', 'import', 'default'].includes(condition)) {
            const entry = importEntry(next);
            if (entry) return entry;
        }
    }
}

// Extract verified npm artifacts into a caller-owned scratch directory.
export function pinnedModules(archives, root) {
    const installed = new Map();
    function install(name) {
        if (installed.has(name)) return installed.get(name);
        const pin = pins.get(name);
        assert.ok(pin, `missing npm pin: ${name}`);
        const file = `${name.split('/').at(-1)}-${pin.version}.tgz`;
        let archive = join(archives, name.replace('/', '__'), file);
        if (!existsSync(archive)) {
            const download = join(root, 'archives', name.replace('/', '__'));
            mkdirSync(download, {recursive: true});
            archive = join(download, file);
            execFileSync('curl', ['--proto', '=https', '-fsSL', '--retry', '2', '-o', archive, pin.tarball]);
        }
        const digest = createHash('sha512').update(readFileSync(archive)).digest('base64');
        assert.equal(`sha512-${digest}`, pin.integrity, name);
        const dir = join(root, 'node_modules', name);
        mkdirSync(dir, {recursive: true});
        execFileSync('tar', ['-xf', archive, '--strip-components=1', '-C', dir]);
        const path = join(dir, 'package.json');
        const pkg = JSON.parse(readFileSync(path));
        assert.equal(pkg.name, name);
        assert.equal(pkg.version, pin.version);
        assert.equal(pkg.license, pin.license);
        const scoped = pkg.exports?.['.'] ?? pkg.exports;
        const entry = importEntry(scoped) ?? pkg.module ?? pkg.main;
        const url = entry && pathToFileURL(realpathSync(join(dir, entry)));
        const info = {path, url};
        installed.set(name, info);
        for (const dep of Object.keys(pkg.dependencies ?? {})) {
            const dependency = install(dep);
            assert.equal(realpathSync(findPackageJSON(dep, pathToFileURL(path))), dependency.path);
        }
        return info;
    }
    return {pins, installed, install, load(name) {
        const {url} = install(name);
        assert.ok(url, `${name} ESM entry`);
        return import(url);
    }};
}
