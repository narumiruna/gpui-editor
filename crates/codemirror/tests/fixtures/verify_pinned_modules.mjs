// node crates/codemirror/tests/fixtures/verify_pinned_modules.mjs [archive-root]
import assert from 'node:assert/strict';
import {execFileSync} from 'node:child_process';
import {mkdtempSync, realpathSync, rmSync, writeFileSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {pinnedModules} from './pinned_modules.mjs';

const root = realpathSync(mkdtempSync(join(tmpdir(), 'codemirror-dependencies-')));
try {
    const modules = pinnedModules(process.argv[2] ? resolve(process.argv[2]) : root, root);
    for (const name of modules.pins.keys()) modules.install(name);
    assert.equal(modules.installed.size, modules.pins.size);
    writeFileSync(join(root, 'package.json'), JSON.stringify({private: true,
        dependencies: Object.fromEntries([...modules.pins.values()].map(pin => [pin.name, pin.version]))}));
    execFileSync('npm', ['ls', '--all', '--omit=dev', '--json'], {cwd: root, stdio: ['ignore', 'pipe', 'inherit']});
    const {LSPClient, signatureHelp} = await modules.load('@codemirror/lsp-client');
    assert.equal(typeof LSPClient, 'function');
    assert.equal(typeof signatureHelp, 'function');
    console.log(`Verified ${modules.installed.size} SHA-512 archives and the npm dependency graph.`);
} finally {
    rmSync(root, {recursive: true, force: true});
}
