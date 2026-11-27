// Run with capture_browser_fixture.mjs, package @codemirror/merge and the SHA-512-verified archives.
import {
    MergeView, acceptChunk, getChunks, getOriginalDoc, goToNextChunk, originalDocChangeEffect,
    uncollapseUnchanged, unifiedMergeView,
} from '@codemirror/merge';
import {ChangeSet, Compartment} from '@codemirror/state';
import {EditorView} from '@codemirror/view';

function capture() {
    const a = 'start\nold 🍎\nkeep 1\nkeep 2\nkeep 3\nkeep 4\nkeep 5\nkeep 6\nend A\nfar 1\nfar 2\nfar 3\nfar 4';
    const b = a.replace('old 🍎', 'new 🍋\nextra').replace('end A', 'end B');
    const collapse = {margin: 1, minSize: 2};
    const common = [
        {op: 'click', at: 28},
        {op: 'edit', side: 'b', from: 0, to: 0, insert: '😀 '},
        {op: 'next'},
    ];
    const tail = [
        {op: 'edit', side: 'a', from: 79, to: 80, insert: '🫐'},
        {op: 'edit', side: 'b', from: 90, to: 91, insert: '🫐'},
    ];
    const cases = [
        {name: 'unified collapse persists until a chunk touches it', a, b, collapse, steps: [
            ...common, {op: 'same'}, {op: 'accept'}, ...tail, {op: 'enable'}, {op: 'disable'},
        ]},
        {name: 'side-by-side collapse follows the same edits', split: true, a, b, collapse,
            steps: [common[0], {op: 'orientation'}, {op: 'same'}, ...common, ...tail, {op: 'disable'}, {op: 'enable'}]},
        {name: 'zero margin keeps a touching range until chunks update', a: 'a\nold\n🍎\nc\nd',
            b: 'a\nnew\n🍎\nc\nd', collapse: {margin: 0, minSize: 1}, steps: [
                {op: 'expand', at: 10}, {op: 'edit', side: 'b', from: 2, to: 5, insert: 'NEW'},
            ]},
        {name: 'a merged expand effect follows its range and editor edits can reveal the rest',
            a, b, collapse, steps: [
                {op: 'edit_and_expand', from: 0, to: 0, insert: '😀 ', at: 28},
                tail[1],
                {op: 'edit', side: 'b', from: 90, to: 94, insert: '3'},
            ]},
    ];

    const encoder = new TextEncoder();
    const byte = (doc, pos) => encoder.encode(doc.sliceString(0, Math.min(pos, doc.length))).length;
    const position = (doc, offset) => {
        let units = 0, count = 0;
        for (const char of doc.toString()) {
            if (count === offset) break;
            count += encoder.encode(char).length;
            units += char.length;
        }
        if (count !== offset) throw Error(`not a byte boundary: ${offset}`);
        return units;
    };
    function collapsed({state}) {
        return state.facet(EditorView.decorations).flatMap(set => {
            const ranges = [];
            if (typeof set !== 'function') set.between(0, state.doc.length, (from, to, value) => {
                if (value.spec.widget?.type === 'collapsed-unchanged-code') {
                    ranges.push([byte(state.doc, from), byte(state.doc, to)]);
                }
            });
            return ranges;
        });
    }

    return cases.map(test => {
        const compartment = new Compartment();
        let extension;
        const pair = test.split ? new MergeView({
            a: {doc: test.a}, b: {doc: test.b}, collapseUnchanged: test.collapse, parent: document.body,
        }) : null;
        const view = pair?.b ?? new EditorView({doc: test.b, parent: document.body,
            extensions: compartment.of(extension = unifiedMergeView({original: test.a, collapseUnchanged: test.collapse})),
        });
        const original = () => pair?.a.state.doc ?? getOriginalDoc(view.state);
        function snapshot() {
            const a = original(), b = view.state.doc;
            const {chunks} = getChunks(view.state);
            return {
                a: a.toString(), b: b.toString(), head: byte(b, view.state.selection.main.head),
                chunks: chunks.map(c => [byte(a, c.fromA), byte(a, c.toA), byte(b, c.fromB), byte(b, c.toB)]),
                collapsed: collapsed(view), ...(pair && {peer: collapsed(pair.a)}),
            };
        }
        const initial = snapshot();
        const oracle = test.steps.map(step => {
            const target = step.side === 'a' ? original() : view.state.doc;
            const changes = () => ({from: position(target, step.from), to: position(target, step.to), insert: step.insert});
            switch (step.op) {
            case 'click': {
                const widget = [...view.contentDOM.querySelectorAll('.cm-collapsedLines')]
                    .find(node => byte(target, view.posAtDOM(node)) === step.at);
                if (!widget) throw Error(`missing widget at byte ${step.at}`);
                widget.click();
                break;
            }
            case 'edit':
                if (step.side === 'b') view.dispatch({changes: changes()});
                else if (pair) pair.a.dispatch({changes: changes()});
                else view.dispatch({effects: originalDocChangeEffect(view.state, ChangeSet.of(changes(), target.length))});
                break;
            case 'expand':
                view.dispatch({effects: uncollapseUnchanged.of(position(target, step.at))});
                break;
            case 'edit_and_expand':
                view.dispatch({changes: changes()}, {effects: uncollapseUnchanged.of(position(target, step.at))});
                break;
            case 'next':
                if (!goToNextChunk(view)) throw Error('navigation failed');
                break;
            case 'accept':
                if (!acceptChunk(view)) throw Error('accept failed');
                break;
            case 'same':
                if (pair) pair.reconfigure({collapseUnchanged: test.collapse});
                else view.dispatch({effects: compartment.reconfigure(extension)});
                break;
            case 'orientation':
                pair.reconfigure({orientation: 'b-a'});
                break;
            case 'enable':
            case 'disable': {
                const collapseUnchanged = step.op === 'enable' ? test.collapse : undefined;
                if (pair) pair.reconfigure({collapseUnchanged});
                else {
                    extension = unifiedMergeView({original: original(), collapseUnchanged});
                    view.dispatch({effects: compartment.reconfigure(extension)});
                }
                break;
            }
            default:
                throw Error(`unknown step: ${step.op}`);
            }
            return snapshot();
        });
        (pair ?? view).destroy();
        return {...test, initial, oracle};
    });
}

globalThis.nativeSupport = capture().map(row => JSON.stringify(row));
