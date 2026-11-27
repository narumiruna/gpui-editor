// Run in a trusted headless browser with the manifest-verified pinned
// LSP, state, and view packages. The runner collects nativeSupport as JSONL.
// The in-memory peer never opens a file or contacts an external server.
import { EditorView, keymap } from '@codemirror/view';
import { LSPClient, closeReferencePanel, findReferences, findReferencesKeymap } from '@codemirror/lsp-client';

const assert = (condition, message) => { if (!condition) throw Error(message); };
const tick = () => new Promise(resolve => setTimeout(resolve, 0));
const REF = 'textDocument/references';
const EXTERNAL = 'https://example.invalid/not-open';
const uri = 'file:///workspace/src/main.rs', otherUri = 'file:///workspace/lib/é.rs';
const doc = 'head\n😀alpha = alpha\ntail', otherDoc = 'é\n  alpha';
const loc = (uri, line, from, to) => ({uri, range: {start: {line, character: from}, end: {line, character: to}}});
const first = loc(uri, 1, 2, 7), second = loc(uri, 1, 10, 15), other = loc(otherUri, 1, 2, 7), unknown = loc(EXTERNAL, 0, 0, 1);
const mixed = [unknown, first, other, unknown, other, second];

async function peer(capabilities) {
    let receive;
    const requests = [];
    const client = new LSPClient();
    const reply = (id, result) => receive(JSON.stringify({jsonrpc: '2.0', id, result}));
    await client.connect({subscribe(fn) { receive = fn; }, unsubscribe() {}, send(raw) {
        const message = JSON.parse(raw);
        if (message.method === 'initialize') queueMicrotask(() => reply(message.id, {capabilities}));
        else if (message.method === REF) requests.push(message);
    }});
    return {client, reply, requests};
}

const make = (doc, extensions = []) => new EditorView({doc, parent: document.body, extensions});
const unbound = make(doc, [keymap.of(findReferencesKeymap)]);
const off = await peer({referencesProvider: false});
const unsupported = make(doc, [off.client.plugin(uri, 'rust')]);
const key = (node, key, keyCode, shiftKey = false) => {
    const event = new KeyboardEvent('keydown', {key, keyCode, shiftKey, bubbles: true, cancelable: true});
    node.dispatchEvent(event);
    return event.defaultPrevented;
};
const result = {
    browser: navigator.userAgent.match(/(?:HeadlessChrome|Chrome)\/([\d.]+)/)?.[1],
    uri, otherUri, doc, otherDoc,
    keys: findReferencesKeymap.map(({key, preventDefault}) => [key, !!preventDefault]),
    withoutPlugin: [findReferences(unbound), closeReferencePanel(unbound), key(unbound.contentDOM, 'Escape', 27)],
    withoutCapability: [findReferences(unsupported), closeReferencePanel(unsupported)],
    steps: [],
};
assert(!off.requests.length, 'unsupported command sent request');
unbound.destroy();
unsupported.destroy();
off.client.disconnect();

const {client, requests, reply} = await peer({referencesProvider: {}, textDocumentSync: {openClose: true, change: 2}});
const view = make(doc, [client.plugin(uri, 'rust'), keymap.of(findReferencesKeymap)]);
view.dispatch({selection: {anchor: 0, head: 8}});
const otherView = make(otherDoc, [client.plugin(otherUri, 'rust')]);
const ids = new Map();
const panel = () => view.dom.querySelector('.cm-lsp-reference-panel');
const item = '.cm-lsp-reference';
const selected = () => panel() ? [...panel().querySelectorAll(item)].findIndex(row => row.hasAttribute('aria-selected')) : null;
const pos = view => new TextEncoder().encode(view.state.sliceDoc(0, view.state.selection.main.head)).length;
function record(name, fields = {}) {
    const dom = panel();
    if (dom && !ids.has(dom)) ids.set(dom, ids.size);
    result.steps.push({name, ...fields, panel: dom ? ids.get(dom) : null, selected: selected()});
}
function display() {
    return [...panel().children].map(child => {
        if (child.className === 'cm-lsp-reference-file') return {header: child.textContent};
        const line = child.querySelector('span'), matched = child.querySelector('strong');
        return {line: Number.parseInt(line.textContent), before: line.nextSibling === matched ? '' : line.nextSibling.textContent,
            matched: matched.textContent, after: matched.nextSibling?.textContent ?? ''};
    });
}
async function send() {
    await tick();
    const request = requests.shift();
    assert(request, 'missing reference request');
    assert(!requests.length, 'unexpected request');
    return request;
}
async function response(name, wire, extra) {
    const handled = findReferences(view);
    const request = await send();
    reply(request.id, wire);
    await tick();
    record(name, {handled, result: wire, ...extra?.()});
    return request.params;
}
record('close-initial', {handled: closeReferencePanel(view)});
for (const [name, wire] of [['null-closed', null], ['empty-closed', []], ['unknown-closed', [unknown]]]) await response(name, wire);
result.params = await response('open-mixed', mixed, () => ({display: display(), bottom: !!panel().closest('.cm-panels-bottom'), panelFocused: document.activeElement === panel()}));
const panelKey = (name, event, code) => record(name, {key: event, handled: key(panel(), event, code)});
panelKey('arrow-up-wrap', 'ArrowUp', 38);
panelKey('page-down-wrap', 'PageDown', 34);
panelKey('arrow-down', 'ArrowDown', 40);
panelKey('page-up', 'PageUp', 33);
panelKey('end', 'End', 35);
panelKey('end-again', 'End', 35);
panelKey('home', 'Home', 36);
panelKey('ignored-key', 'Tab', 9);
panelKey('move-before-no-op', 'ArrowDown', 40);
for (const [name, wire] of [['null-open', null], ['empty-open', []], ['unknown-open', [unknown]]]) await response(name, wire);
await response('replace-identical', mixed, () => ({display: display(), panelFocused: document.activeElement === panel()}));
panelKey('move-before-replacement', 'End', 35);
await response('replace-with-one', [second], () => ({display: display()}));
panelKey('single-row-wrap', 'ArrowDown', 40);
const firstPosition = pos(view);
const handledSpace = key(panel(), ' ', 32);
await tick();
record('space', {key: ' ', handled: handledSpace, position: pos(view), previous: firstPosition});
const inserted = 'é\n';
view.dispatch({changes: {from: 0, insert: inserted}});
record('edit-before-activation', {inserted, display: display()});
const handledEnter = key(panel(), 'Enter', 13);
await tick();
record('enter', {key: 'Enter', handled: handledEnter, position: pos(view), editorFocused: view.hasFocus});
const handledEscape = key(panel(), 'Escape', 27);
record('escape-in-panel', {key: 'Escape', handled: handledEscape, editorFocused: view.hasFocus});
record('close-after-escape', {handled: closeReferencePanel(view)});
const shiftedFirst = loc(uri, 2, 2, 7);
await response('open-before-pending', [shiftedFirst], () => ({display: display()}));
const pending = key(view.contentDOM, 'F12', 123, true);
const pendingRequest = await send();
record('close-while-pending', {handled: closeReferencePanel(view), pending});
reply(pendingRequest.id, [shiftedFirst]);
await tick();
record('reply-after-close', {result: [shiftedFirst], display: display()});
record('close-open', {handled: closeReferencePanel(view)});
record('close-again', {handled: closeReferencePanel(view)});
await response('reopen', [other], () => ({display: display()}));
record('escape-in-editor', {handled: key(view.contentDOM, 'Escape', 27)});
record('escape-in-editor-closed', {handled: key(view.contentDOM, 'Escape', 27)});

const shiftedSecond = loc(uri, 2, 10, 15);
await response('open-for-pointer', [shiftedFirst, shiftedSecond]);
const pointer = new MouseEvent('click', {bubbles: true, cancelable: true, button: 0});
panel().querySelectorAll(item)[1].querySelector('strong').dispatchEvent(pointer);
await tick();
record('pointer-row', {click: 1, handled: pointer.defaultPrevented, position: pos(view), editorFocused: view.hasFocus});
assert(pointer.defaultPrevented && selected() === 1 && view.hasFocus, 'pointer activation');

view.destroy();
otherView.destroy();
client.disconnect();
const {steps, ...meta} = result;
globalThis.nativeSupport = [
    JSON.stringify({...meta, package: '@codemirror/lsp-client@6.3.1',
        integrity: 'sha512-8uCLbozDGJKxEHKkPy3np4dNOvN1sLkRogsFkqm9QSK5nWwPktiqIeh+c9PWD1P3PUwnCgwfS1YWWNuuyts/dw=='}),
    ...steps.map(step => JSON.stringify(step)),
];
