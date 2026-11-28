// Capture with capture_browser_fixture.mjs, Chrome, and @codemirror/lang-json.
import {EditorView} from '@codemirror/view';
import {Compartment} from '@codemirror/state';
import {Language, ensureSyntaxTree, forceParsing, syntaxTree, syntaxTreeAvailable} from '@codemirror/language';
import {jsonLanguage} from '@codemirror/lang-json';
import {NodeProp} from '@lezer/common';

const idle = new Map();
let next = 0;
globalThis.requestIdleCallback = fn => { idle.set(++next, fn); return next; };
globalThis.cancelIdleCallback = id => idle.delete(id);
Date.now = () => 4_000_000;
const marker = new NodeProp();
const configure = label => jsonLanguage.configure({props: [marker.add({String: label})]});
const first = configure('first'), second = configure('second');
const slot = new Compartment();
const parent = document.body.appendChild(document.createElement('div'));
const source = '[\n' + '  "hi",\n'.repeat(1500) + '  null\n]';
const view = new EditorView({doc: source, parent, extensions: [slot.of(first), EditorView.theme({
    '&': {height: '140px'}, '.cm-scroller': {overflow: 'auto'},
})]});
const rows = [];
const label = tree => tree.topNode.getChild('Array')?.getChild('String')?.type.prop(marker) ?? null;
const snapshot = (event, extra = {}) => {
    const state = view.state;
    const tree = syntaxTree(state);
    rows.push(JSON.stringify({event, length: state.doc.length, viewport: view.viewport.to,
        rootEnd: tree.length, through: state.field(Language.state, false)?.context.treeLen ?? 0,
        marker: label(tree), complete: syntaxTreeAvailable(state), ...extra}));
};
try {
    snapshot('install');
    const initial = view.state, original = syntaxTree(initial);
    const stage = ensureSyntaxTree(view.state, 5000, 10);
    snapshot('ensure', {stage: stage?.length, stageMarker: stage && label(stage),
        unchanged: initial === view.state, available: syntaxTreeAvailable(view.state, 5000)});
    const forced = forceParsing(view, 5000, 10);
    snapshot('force', {forced, sameStage: syntaxTree(view.state) === stage,
        retained: syntaxTree(initial) === original});
    await new Promise(resolve => setTimeout(resolve, 120));
    const callbacks = [...idle.values()];
    idle.clear();
    for (const callback of callbacks) callback({timeRemaining: () => 100});
    snapshot('idle', {callbacks: callbacks.length});
    const full = syntaxTree(view.state);
    view.dispatch({selection: {anchor: 11}});
    snapshot('selection', {sameTree: syntaxTree(view.state) === full});
    view.dispatch({effects: slot.reconfigure(second)});
    snapshot('reconfigure', {retainedMarker: label(full)});
    view.dispatch({effects: slot.reconfigure([])});
    snapshot('detach');
} finally {
    view.destroy();
    parent.remove();
}
globalThis.nativeSupport = rows;
