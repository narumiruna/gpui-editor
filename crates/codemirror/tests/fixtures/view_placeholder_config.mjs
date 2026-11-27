// Run against the integrity-checked browser import map for the pinned packages.
import {Compartment, Prec} from '@codemirror/state';
import {EditorView, placeholder} from '@codemirror/view';

const compartment = new Compartment();
const shared = placeholder('second');
const labels = [Prec.low(placeholder('last')), shared, shared, Prec.high(placeholder(''))];
const view = new EditorView({parent: document.body, extensions: compartment.of(labels)});
function snapshot(label) {
    const providers = view.state.facet(EditorView.decorations);
    const widgetSets = providers.flatMap(provider => {
        const cursor = provider(view).iter();
        return cursor.value ? [[cursor.from, cursor.to, cursor.value.spec.widget.content]] : [];
    });
    const rendered = [...view.dom.querySelectorAll('.cm-placeholder')].map(node =>
        [node.textContent, node.getAttribute('aria-hidden'), node.style.pointerEvents]);
    return {label, doc: view.state.doc.toString(), aria: view.contentDOM.getAttribute('aria-placeholder'), widgetSets, rendered};
}
const rows = [snapshot('multiple')];
view.dispatch({changes: {from: 0, insert: 'typed'}});
rows.push(snapshot('typed'));
view.dispatch({changes: {from: 0, to: 5}, effects: compartment.reconfigure(placeholder('new'))});
rows.push(snapshot('replaced'));
view.dispatch({effects: compartment.reconfigure([])});
rows.push(snapshot('removed'));
view.destroy();
globalThis.nativeSupport = rows.map(row => JSON.stringify(row));
