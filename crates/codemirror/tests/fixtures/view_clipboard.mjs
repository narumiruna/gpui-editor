import { EditorSelection, EditorState, Prec, Transaction } from '@codemirror/state';
import { EditorView } from '@codemirror/view';

// Use a fresh DataTransfer for each event. Do not access the OS clipboard.
const rows = [];
const cursors = (...positions) => positions.map(at => [at, at]);
const copy = view => [view, 'copy'];
const cut = view => [view, 'cut'];
const paste = (view, text) => [view, 'paste', text];
const record = (state, tr, clipboard, handled, log) => ({
    doc: state.doc.toString(),
    ranges: state.selection.ranges.map(r => [r.anchor, r.head, r.assoc]),
    main: state.selection.mainIndex,
    event: tr?.annotation(Transaction.userEvent) ?? null,
    scroll: tr?.scrollIntoView ?? false,
    clipboard,
    handled,
    log,
});
const transforms = {
    prefix: text => '>' + text,
    main: (text, state) => `${text}:${state.selection.mainIndex}`,
    upper: text => text.toUpperCase(),
    drop: () => '',
    unprefix: text => text.replace(/^#/, ''),
};
const priority = { high: Prec.high, low: Prec.low };

async function capture(name, editors, ops) {
    // Clear upstream's module-wide linewise marker with a handled ordinary copy.
    const reset = new EditorView({ doc: 'x', selection: { anchor: 0, head: 1 }, parent: document.body });
    reset.focus();
    const clear = new ClipboardEvent('copy', { clipboardData: new DataTransfer(), cancelable: true, bubbles: true });
    if (reset.contentDOM.dispatchEvent(clear)) throw new Error('Copy did not clear the marker');
    reset.destroy();

    const views = editors.map(spec => {
        const observation = { log: [], tr: null };
        const extensions = [EditorState.allowMultipleSelections.of(true)];
        if (spec.sep) extensions.push(EditorState.lineSeparator.of(spec.sep));
        if (spec.readOnly) extensions.push(EditorState.readOnly.of(true));
        for (const [kind, facet] of [['input', EditorView.clipboardInputFilter], ['output', EditorView.clipboardOutputFilter]]) {
            const filters = new Map();
            for (const [name, precedence] of spec[kind] ?? []) {
                if (!filters.has(name)) {
                    const ext = facet.of((text, state) => {
                        observation.log.push([kind, name, text, state.doc.toString(), state.selection.mainIndex]);
                        return transforms[name](text, state);
                    });
                    filters.set(name, priority[precedence]?.(ext) ?? ext);
                }
                extensions.push(filters.get(name));
            }
        }
        const view = new EditorView({
            doc: spec.doc,
            selection: EditorSelection.create(spec.ranges.map(([a, b, assoc]) => EditorSelection.range(a, b, undefined, undefined, assoc)), spec.main ?? 0),
            parent: document.body,
            extensions,
            dispatchTransactions(trs, view) {
                if (trs.length !== 1 || observation.tr) throw new Error('Unexpected dispatch');
                [observation.tr] = trs;
                view.update(trs);
            },
        });
        return { view, observation };
    });
    const results = [];
    for (const [index, kind, text] of ops) {
        const { view, observation } = views[index];
        observation.log = [];
        observation.tr = null;
        view.focus();
        if (document.activeElement !== view.contentDOM) throw new Error('Editor did not get DOM focus');
        const clipboardData = new DataTransfer();
        if (kind === 'paste') clipboardData.setData('text/plain', text);
        const evt = new ClipboardEvent(kind, { clipboardData, bubbles: true, cancelable: true });
        const handled = !view.contentDOM.dispatchEvent(evt);
        const clipboard = kind !== 'paste' && handled ? clipboardData.getData('text/plain') : null;
        results.push(record(view.state, observation.tr, clipboard, handled, observation.log));
    }
    for (const { view } of views) view.destroy();
    rows.push(JSON.stringify({ case: name, editors, ops, results }));
}

await capture('selected_join_order_and_cut', [
    { doc: 'éA\r\n🦀B\r\nxy\r\nlast', sep: '\r\n', ranges: [[3, 0], [6, 6], [7, 9]], main: 2,
        output: [['upper', 'low'], ['main'], ['prefix', 'high'], ['main']] },
], [copy(0), cut(0)]);

await capture('linewise_copy_and_cut_ranges', [
    { doc: 'one\ntwo\nthree\nlast', ranges: cursors(1, 2, 6, 15), main: 3 },
], [copy(0), cut(0)]);

await capture('linewise_paste_skips_duplicate_lines', [
    { doc: 'one\ntwo\nthree\nfour', ranges: cursors(1, 5, 9) },
    { doc: 'aaa\nbbb\nccc', ranges: [[0, 0], [2, 2, 1], [5, 5]], main: 1 },
], [copy(0), paste(1, 'one\ntwo\nthree')]);

await capture('linewise_paste_repeats_all_lines_on_count_mismatch', [
    { doc: 'one\ntwo\nthree', ranges: cursors(1, 5) },
    { doc: 'aaa\nbbb', ranges: cursors(1, 2, 5) },
], [copy(0), paste(1, 'one\r\ntwo')]);

for (const sep of ['\r\n', '||']) {
    await capture(sep === '||' ? 'custom_separator_marker' : 'crlf_marker', [
        { doc: ['one', 'two'].join(sep), sep, ranges: cursors(1, 5) },
        { doc: ['aaa', 'bbb'].join(sep), sep, ranges: cursors(1, 5) },
    ], [copy(0), paste(1, ['one', 'two'].join(sep))]);
}

await capture('declined_copy_keeps_marker_and_ordinary_copy_clears_it', [
    { doc: 'one', ranges: cursors(1) },
    { doc: 'drop', ranges: [[0, 4]], output: [['drop']] },
    { doc: 'aaa', ranges: cursors(1) },
    { doc: 'one', ranges: [[0, 3]] },
    { doc: 'aaa', ranges: cursors(1) },
], [copy(0), cut(1), paste(2, 'one'), copy(3), paste(4, 'one')]);

await capture('read_only_cut_keeps_marker_without_edit', [
    { doc: 'one', ranges: cursors(1), readOnly: true, input: [['prefix']], output: [['upper']] },
    { doc: 'aaa', ranges: cursors(1), input: [['unprefix']] },
], [cut(0), paste(0, 'skip'), paste(1, '#ONE')]);

await capture('input_filters_run_before_break_parsing', [
    { doc: 'abc\ndef\nghi', ranges: [[0, 1], [4, 5], [8, 9]],
        input: [['upper', 'low'], ['unprefix', 'high'], ['upper', 'low']] },
], [paste(0, '#a\rb\r\n')]);

await capture('empty_linewise_copy_and_paste', [
    { doc: '', ranges: cursors(0) },
    { doc: 'aaa', ranges: cursors(1) },
    { doc: 'abc\nkeep', ranges: cursors(1), output: [['drop']] },
], [cut(0), paste(1, ''), cut(2)]);

await capture('ordinary_paste_repeats_full_text_on_count_mismatch', [
    { doc: 'ab\ncd', ranges: [[0, 1], [4, 4]], main: 1 },
], [paste(0, '🦀\rA\r\nB')]);

await capture('linewise_marker_does_not_override_nonempty_selection', [
    { doc: 'one\ntwo', ranges: cursors(1, 5) },
    { doc: 'aaa\nbbb', ranges: [[1, 2], [5, 5]] },
], [copy(0), paste(1, 'one\ntwo')]);

await capture('single_linewise_copy_uses_target_separator', [
    { doc: 'one', ranges: cursors(1) },
    { doc: 'aaa||bbb', sep: '||', ranges: cursors(1, 5) },
], [copy(0), paste(1, 'one')]);

await capture('linewise_cut_and_paste_on_mixed_breaks', [
    { doc: '[\r  "猫",\r  "dog"\r\n]', ranges: cursors(5) },
], [cut(0), paste(0, '  "猫",')]);

globalThis.nativeSupport = rows;
