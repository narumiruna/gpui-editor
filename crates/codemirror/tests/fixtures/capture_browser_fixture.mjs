// node crates/codemirror/tests/fixtures/capture_browser_fixture.mjs <archive-root> <chrome-executable> <comma-separated-packages> <oracle.mjs>
import assert from 'node:assert/strict';
import {execFile} from 'node:child_process';
import {mkdtempSync, readFileSync, realpathSync, rmSync} from 'node:fs';
import {createServer} from 'node:http';
import {tmpdir} from 'node:os';
import {join, relative, resolve} from 'node:path';
import {fileURLToPath} from 'node:url';
import {promisify} from 'node:util';
import {pinnedModules} from './pinned_modules.mjs';

const [archiveRoot, chrome, upstream, oracle] = process.argv.slice(2);
assert.ok(archiveRoot && chrome && upstream && oracle, 'supply the archive root, Chrome executable, packages and oracle path');
const root = realpathSync(mkdtempSync(join(tmpdir(), 'browser-fixture-')));
const server = createServer();
try {
    const {install, installed} = pinnedModules(resolve(archiveRoot), root);
    for (const name of upstream.split(',')) install(name);
    const imports = {};
    const files = new Map();
    for (const [name, {url}] of installed) {
        assert.ok(url, `${name} has a browser module`);
        const path = fileURLToPath(url);
        const route = '/' + relative(root, path);
        imports[name] = route;
        files.set(route, readFileSync(path));
    }
    files.set('/oracle.mjs', readFileSync(resolve(oracle)));
    const page = `<!doctype html><meta charset="utf-8">
<script type="importmap">${JSON.stringify({imports})}</script>
<body><script type="module">
const output = document.createElement('pre');
output.id = 'capture';
try {
    await import('./oracle.mjs');
    const newline = String.fromCharCode(10);
    const bytes = new TextEncoder().encode(globalThis.nativeSupport.join(newline) + newline);
    output.textContent = btoa(String.fromCharCode(...bytes));
} catch (error) {
    output.textContent = error.stack;
}
document.body.append(output);
</script></body>`;
    server.on('request', (request, response) => {
        if (request.url === '/') {
            response.writeHead(200, {'Content-Type': 'text/html'}).end(page);
        } else if (files.has(request.url)) {
            response.writeHead(200, {'Content-Type': 'text/javascript'}).end(files.get(request.url));
        } else {
            response.writeHead(404).end();
        }
    });
    await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
    const {stdout} = await promisify(execFile)(chrome, [
        '--headless', '--disable-gpu', '--disable-background-networking', '--no-first-run',
        '--no-default-browser-check', '--dump-dom', '--virtual-time-budget=2000',
        `--user-data-dir=${join(root, 'profile')}`, `http://127.0.0.1:${server.address().port}`,
    ], {timeout: 15000, maxBuffer: 1024 * 1024});
    const capture = /<pre id="capture">(.*?)<\/pre>/s.exec(stdout)?.[1];
    assert.match(capture ?? '', /^[a-zA-Z0-9+/=]+$/u, capture || 'missing browser capture');
    process.stdout.write(Buffer.from(capture, 'base64'));
} finally {
    server.close();
    rmSync(root, {recursive: true, force: true});
}
