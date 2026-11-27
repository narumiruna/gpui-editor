// Capture with capture_browser_fixture.mjs, Chrome, and @codemirror/lang-wast.
import {EditorView} from '@codemirror/view';
import {Compartment} from '@codemirror/state';
import {Language, defineLanguageFacet, ensureSyntaxTree, forceParsing, sublanguageProp, syntaxParserRunning, syntaxTree, syntaxTreeAvailable} from '@codemirror/language';
import {wast, wastLanguage} from '@codemirror/lang-wast';
import {NodeProp} from '@lezer/common';
import {highlightTree, styleTags, tagHighlighter, tags} from '@lezer/highlight';

const callbacks = new Map();
const canceled = [];
let next = 0;
let inputPending = false;
globalThis.requestIdleCallback = callback => {
    callbacks.set(++next, callback);
    return next;
};
globalThis.cancelIdleCallback = id => {
    canceled.push(id);
    callbacks.delete(id);
};
Object.defineProperty(navigator, 'scheduling', {
    configurable: true,
    value: {isInputPending: () => inputPending},
});
Date.now = () => 4_000_000;

const rows = [];
const delay = () => new Promise(resolve => setTimeout(resolve, 120));
async function deliverIdle() {
    await delay();
    const pending = [...callbacks.values()];
    callbacks.clear();
    for (const callback of pending) callback({timeRemaining: () => 100});
    return pending.length;
}

function createView(language = wast()) {
    const parent = document.body.appendChild(document.createElement('div'));
    parent.style.cssText = 'height:160px; width:600px';
    const slot = new Compartment();
    const view = new EditorView({
        doc: '(func $f)\n'.repeat(12_000),
        parent,
        extensions: [slot.of(language), EditorView.theme({
            '&': {height: '140px'},
            '.cm-scroller': {overflow: 'auto'},
        })],
    });
    function record(event, extra = {}) {
        const state = view.state;
        const context = state.field(Language.state, false)?.context;
        const tree = syntaxTree(state);
        rows.push(JSON.stringify({
            event,
            length: state.doc.length,
            vp: view.viewport.to,
            pub: tree.length,
            working: context?.tree.length ?? 0,
            through: context?.treeLen ?? 0,
            inFlight: context?.parse?.parsedPos ?? null,
            full: syntaxTreeAvailable(state),
            visible: syntaxTreeAvailable(state, view.viewport.to),
            running: syntaxParserRunning(view),
            error0: tree.topNode.firstChild?.type.isError ?? false,
            ...extra,
        }));
    }
    return {view, slot, parent, record};
}

{
    const {view, slot, parent, record} = createView();
    try {
        const initial = view.state;
        const initialTree = syntaxTree(initial);
        record('install');
        const ensured = ensureSyntaxTree(view.state, 5000, 10);
        record('ensure', {
            returned: ensured?.length,
            sameState: initial === view.state,
            oldTree: syntaxTree(initial) === initialTree,
            available5000: syntaxTreeAvailable(view.state, 5000),
            available5001: syntaxTreeAvailable(view.state, 5001),
        });
        const forced = forceParsing(view, 5000, 10);
        record('force', {
            forced,
            sameState: initial === view.state,
            oldTree: syntaxTree(initial) === initialTree,
            sameAsEnsure: ensured === syntaxTree(view.state),
        });
        record('idle', {callbacks: await deliverIdle()});
        const previous = view.state;
        const previousTree = syntaxTree(previous);
        view.dispatch({changes: {from: 0, insert: ')'}});
        record('edit', {
            prevPub: syntaxTree(previous).length,
            oldTree: syntaxTree(previous) === previousTree,
            oldDoc: previous.doc.length,
        });
        const editTree = syntaxTree(view.state);
        view.dispatch({selection: {anchor: 1}});
        record('selection', {sameTree: editTree === syntaxTree(view.state)});
        await delay();
        const queued = [...callbacks.keys()];
        view.dispatch({effects: slot.reconfigure([])});
        record('detach', {
            queued,
            canceled: [...canceled],
            after: [...callbacks.keys()],
            retainedEdit: editTree.length,
            ensure: ensureSyntaxTree(view.state, 5000, 10),
        });
    } finally {
        view.destroy();
        parent.remove();
    }
}

{
    const {view, parent, record} = createView();
    try {
        view.dispatch({effects: EditorView.scrollIntoView(110_000, {y: 'start'})});
        record('jump');
        inputPending = true;
        record('idleBlocked', {callbacks: await deliverIdle()});
        inputPending = false;
        record('idleViewport', {callbacks: await deliverIdle()});
        record('idleAhead', {callbacks: await deliverIdle()});
    } finally {
        view.destroy();
        parent.remove();
    }
}

{
    const {view, parent, record} = createView();
    try {
        const staged = ensureSyntaxTree(view.state, 5000, 10);
        view.dispatch({changes: {from: 0, insert: ';; 😀\r\n'}});
        record('editAfterEnsure', {retainedStage: staged?.length});
        view.dispatch({changes: {from: 5006, insert: ';; x\n'}});
        record('editAtWorkingEnd');
    } finally {
        view.destroy();
        parent.remove();
    }
}

{
    const marker = new NodeProp();
    const configure = value => wastLanguage.configure({props: [marker.add({Identifier: value})]});
    const identifier = tree => tree.topNode.getChild('App').getChild('Identifier').type;
    const first = configure('first');
    const second = configure('second');
    const {view, slot, parent, record} = createView(first);
    try {
        const initial = syntaxTree(view.state);
        const initialType = identifier(initial);
        record('propsInstall', {
            marker: initialType.prop(marker),
            sameTypeAsRaw: initialType === identifier(first.parser.parse('(func $f)')),
        });
        const staged = ensureSyntaxTree(view.state, 5000, 10);
        const stagedType = identifier(staged);
        record('propsEnsure', {
            marker: stagedType.prop(marker),
            sameType: stagedType === initialType,
            samePublished: syntaxTree(view.state) === initial,
        });
        view.dispatch({effects: slot.reconfigure(second)});
        const replacement = identifier(syntaxTree(view.state));
        record('propsReconfigure', {
            marker: replacement.prop(marker),
            retainedMarker: stagedType.prop(marker),
            sameType: replacement === initialType,
        });
    } finally {
        view.destroy();
        parent.remove();
    }
}

{
    const language = wastLanguage.configure({props: [styleTags({
        'App/...': tags.emphasis,
        'App/Keyword': tags.atom,
        'App/Identifier!': tags.string,
    })]});
    const highlighter = tagHighlighter(['emphasis', 'atom', 'string', 'paren'].map(name => ({tag: tags[name], class: name})));
    const {view, parent, record} = createView(language);
    function styles(from, to) {
        const spans = [];
        highlightTree(syntaxTree(view.state), highlighter, (start, end, names) => {
            spans.push([start, end, names.split(' ')]);
        }, from, to);
        return spans;
    }
    try {
        record('stylesInstall', {range: [0, 24], spans: styles(0, 24)});
        const from = 3002, to = 3024;
        if (!ensureSyntaxTree(view.state, 5000, 10)) throw new Error('missing staged tree');
        record('stylesEnsure', {range: [from, to], spans: styles(from, to)});
        if (!forceParsing(view, 5000, 10)) throw new Error('missing published tree');
        record('stylesForce', {range: [from, to], spans: styles(from, to)});
    } finally {
        view.destroy();
        parent.remove();
    }
}

{
    const keyword = defineLanguageFacet({scope: 'keyword'});
    const identifier = defineLanguageFacet({scope: 'identifier'});
    const opener = defineLanguageFacet({scope: 'opener'});
    const module = defineLanguageFacet({scope: 'module'});
    const language = wastLanguage.configure({props: [sublanguageProp.add({Module: [
        {type: 'extend', facet: keyword, test: node => node.name === 'Keyword'},
        {type: 'replace', facet: identifier, test: node => node.name === 'Identifier'},
        {type: 'replace', facet: opener, test: node => node.name === '('},
        {type: 'extend', facet: module, test: node => node.name === 'Module'},
    ]})]});
    const {view, slot, parent, record} = createView([
        language,
        language.data.of({scope: 'root'}),
    ]);
    const probes = [[2, -1], [2, 1], [6, -1], [6, 1], [3000, -1], [3000, 1], [3001, -1], [3001, 1], [3002, 1], [3007, 1]];
    function data(state = view.state) {
        return probes.map(([pos, side]) => state.languageDataAt('scope', pos, side));
    }
    try {
        const initial = view.state;
        record('dataInstall', {probes, data: data()});
        if (!ensureSyntaxTree(view.state, 5000, 10)) throw new Error('missing staged tree');
        record('dataEnsure', {data: data()});
        if (!forceParsing(view, 5000, 10)) throw new Error('missing published tree');
        record('dataForce', {data: data(), retained: data(initial)});
        view.dispatch({effects: slot.reconfigure([])});
        record('dataDetach', {data: data(), retained: data(initial)});
    } finally {
        view.destroy();
        parent.remove();
    }
}

globalThis.nativeSupport = rows;
