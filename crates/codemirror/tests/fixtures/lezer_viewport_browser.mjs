// Capture with capture_browser_fixture.mjs, Chrome, and @codemirror/lang-lezer.
import {EditorView} from '@codemirror/view';
import {Compartment} from '@codemirror/state';
import {Language, ensureSyntaxTree, forceParsing, syntaxTree, syntaxTreeAvailable} from '@codemirror/language';
import {lezerLanguage} from '@codemirror/lang-lezer';
import {NodeProp} from '@lezer/common';

const callbacks = new Map();
let next = 0;
globalThis.requestIdleCallback = fn => { callbacks.set(++next, fn); return next; };
globalThis.cancelIdleCallback = id => callbacks.delete(id);
Date.now = () => 4_000_000;
const prop = new NodeProp();
const configure = name => lezerLanguage.configure({props: [prop.add({Literal: name})]});
const slot = new Compartment();
const parent = document.body.appendChild(document.createElement('div'));
const source = '@top Main {\n' + '  "hi"\n'.repeat(1500) + '}';
const view = new EditorView({doc: source, parent, extensions: [slot.of(configure('first')), EditorView.theme({
    '&': {height: '140px'}, '.cm-scroller': {overflow: 'auto'},
})]});
const rows = [];
function marker(tree) {
    const cursor = tree.cursor();
    do {
        if (cursor.name === 'Literal') return cursor.type.prop(prop);
    } while (cursor.next());
    return null;
}
function record(step, extra = {}) {
    const state = view.state;
    const tree = syntaxTree(state);
    rows.push(JSON.stringify({step, length: state.doc.length, viewport: view.viewport.to,
        root: tree.length, coverage: state.field(Language.state, false)?.context.treeLen ?? 0,
        complete: syntaxTreeAvailable(state), marker: marker(tree), ...extra}));
}
try {
    const initial = view.state, original = syntaxTree(initial);
    record('install');
    const staged = ensureSyntaxTree(view.state, 5000, 10);
    record('stage', {sameState: initial === view.state, available: syntaxTreeAvailable(view.state, 5000),
        working: staged?.length, markerAt5000: staged && marker(staged)});
    const forced = forceParsing(view, 5000, 10);
    record('publish', {forced, publishedStage: syntaxTree(view.state) === staged,
        retainedInitial: syntaxTree(initial) === original});
    await new Promise(resolve => setTimeout(resolve, 120));
    const ready = [...callbacks.values()];
    callbacks.clear();
    for (const callback of ready) callback({timeRemaining: () => 100});
    record('idle');
    const full = syntaxTree(view.state);
    view.dispatch({effects: slot.reconfigure(configure('next'))});
    record('swap', {retained: marker(full)});
    view.dispatch({effects: slot.reconfigure([])});
    record('detach');
} finally {
    view.destroy();
    parent.remove();
}
globalThis.nativeSupport = rows;
