// node direct_tag_switch.mjs <pinned-archive-directory>
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {pinnedModules} from './pinned_modules.mjs';

const scratch = mkdtempSync(join(realpathSync(tmpdir()), 'direct-tag-switch-'));
const names = ['state', 'view', 'language', 'lang-html', 'lang-xml'];
const {load} = pinnedModules(resolve(process.argv[2]), scratch);
const [{Compartment, EditorSelection, EditorState, Transaction}, {EditorView}, {language}, {html}, {xml}] =
    await Promise.all(names.map(name => load(`@codemirror/${name}`)))
        .finally(() => rmSync(scratch, {recursive: true, force: true}));

// Run the original state's installed input handlers with a host-supplied default.
const cases = [
    ['html-item', 'html', '<Item|', '>'],
    ['xml-item', 'xml', '<Item|', '>'],
    ['html-mismatch', 'html', '<Item></bad|', '>'],
    ['xml-mismatch', 'xml', '<Item></bad|', '>'],
    ['html-void', 'html', '<br|', '>'],
    ['html-upper-void', 'html', '<BR|', '>'],
    ['xml-void', 'xml', '<br|', '>'],
    ['html-nested-exact', 'html', '<Item><Item|</Item>', '>'],
    ['html-nested-case', 'html', '<ITEM><Item|</Item>', '>'],
    ['xml-nested-exact', 'xml', '<Item><Item|</Item>', '>'],
    ['html-rewritten-caret', 'html', '<Old|', '>', ['! <New>', 7, 7]],
    ['xml-rewritten-caret', 'xml', '<Old|', '>', ['! <New>', 7, 7]],
    ['html-rewritten-token', 'html', '<Old|', '>', ['! <New', 6, 6]],
    ['xml-rewritten-token', 'xml', '<Old|', '>', ['! <New', 6, 6]],
    ['html-rewritten-range', 'html', '<Old|', '>', ['<New>', 4, 5]],
    ['xml-rewritten-range', 'xml', '<Old|', '>', ['<New>', 4, 5]],
    ['html-replace-following-gt', 'html', '<Item|>', '>'],
    ['xml-replace-following-gt', 'xml', '<Item|>', '>'],
    ['html-slash', 'html', '<Item><|', '/'],
    ['xml-slash', 'xml', '<Item><|', '/'],
    ['html-raw-mount', 'html', '<script|', '>'],
    ['html-raw-space', 'html', '<script |', '>'],
    ['html-non-mount', 'html', '<script type="text/plain"|', '>'],
    ['html-raw-body', 'html', '<script>x|</script>', '>', ['<New>', 5, 5]],
    ['xml-script', 'xml', '<script|', '>'],
];
const policy = {html, xml};
const snapshot = state => ({doc: state.doc.toString(), selection: [state.selection.main.anchor, state.selection.main.head]});
const asStep = tr => {
    const changes = [];
    tr.changes.iterChanges((from, to, _fromB, _toB, insert) => changes.push([from, to, insert.toString()]));
    return {start: snapshot(tr.startState), end: snapshot(tr.state), changes, event: tr.annotation(Transaction.userEvent)};
};

for (const [name, old, marked, typed, rewrite] of cases) {
    const next = old === 'html' ? 'xml' : 'html';
    const slot = new Compartment();
    const at = marked.indexOf('|');
    const doc = marked.replace('|', '');
    const effect = slot.reconfigure(policy[next]());
    const start = EditorState.create({doc, selection: {anchor: at}, extensions: [
        slot.of(policy[old]()),
        EditorState.transactionFilter.of(tr => {
            if (!rewrite || !tr.isUserEvent('input.type')) return tr;
            const [insert, anchor, head] = rewrite;
            return Transaction.create(tr.startState, tr.startState.changes({from: 0, to: doc.length, insert}),
                EditorSelection.single(anchor, head), tr.effects, tr.annotations, tr.scrollIntoView);
        }),
    ]});
    let base, transactions;
    const view = {state: start, composing: false, dispatch: steps => { transactions = steps; }};
    const getDefault = () => base ??= start.update({changes: {from: at, insert: typed},
        selection: {anchor: at + 1}, effects: effect, userEvent: 'input.type', scrollIntoView: true});
    const handled = start.facet(EditorView.inputHandler).some(run => run(view, at, at, typed, getDefault));
    const accepted = getDefault().state;
    if (handled !== !!transactions || accepted.facet(language) !== policy[next]().language) {
        throw Error(name);
    }
    const steps = transactions ?? [base];
    console.log(JSON.stringify({name, old, doc, at, typed, ...(rewrite ? {rewrite} : {}), steps: steps.map(asStep)}));
}
