// node crates/codemirror/tests/fixtures/lsp_deferred_wire.mjs <archive-root>
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {pinnedModules} from './pinned_modules.mjs';

const root = realpathSync(mkdtempSync(join(tmpdir(), 'lsp-deferred-wire-')));
try {
    const {load} = pinnedModules(resolve(process.argv[2]), root);
    const {LSPClient} = await load('@codemirror/lsp-client');
    const wire = [];
    let receive;
    const transport = {
        subscribe(callback) { receive = callback; },
        unsubscribe(callback) { assert.equal(callback, receive); },
        send(raw) { wire.push(JSON.parse(raw)); },
    };
    const upstream = new LSPClient({timeout: 3000});
    upstream.connect(transport);
    const method = 'workspace/didChangeConfiguration';
    upstream.notification(method, {settings: {phase: 'before'}});
    const pending = upstream.request('textDocument/hover', {
        textDocument: {uri: 'file:///deferred.rs'}, position: {line: 0, character: 4},
    });
    upstream.notification(method, {settings: {phase: 'after'}});
    const tuple = message => [message.method, message.id ?? null, message.params?.settings?.phase ?? null];
    const beforeReady = wire.map(tuple);
    assert.deepEqual(beforeReady, [['initialize', 1, null]]);
    receive(JSON.stringify({jsonrpc: '2.0', id: 1, result: {capabilities: {hoverProvider: true}}}));
    await upstream.initializing;
    // Let all initialization continuations run before sending the feature reply.
    await Promise.resolve();
    const afterReady = wire.slice(1).map(tuple);
    assert.deepEqual(afterReady, [
        ['initialized', null, null], [method, null, 'before'],
        ['textDocument/hover', 2, null], [method, null, 'after'],
    ]);
    receive(JSON.stringify({jsonrpc: '2.0', id: 2, result: null}));
    assert.equal(await pending, null);
    upstream.disconnect();
    console.log(JSON.stringify({beforeReady, afterReady}));
} finally {
    rmSync(root, {recursive: true, force: true});
}
