import { basicSetup } from 'codemirror';
import { javascript } from '@codemirror/lang-javascript';
import { Compartment, EditorSelection, EditorState } from '@codemirror/state';
import { oneDarkTheme } from '@codemirror/theme-one-dark';
import { EditorView, highlightActiveLine, highlightActiveLineGutter, lineNumbers } from '@codemirror/view';

const source = "const label = 'café'; // this logical line wraps across several measured display rows\n\nconst end = 42;\n";
const fixedSelection = EditorSelection.create([EditorSelection.range(6, 11), EditorSelection.cursor(source.indexOf('\n\n') + 1)]);
const blur = document.body.appendChild(document.createElement('button'));
const result = [];
const equal = (a, b, what) => {
    if (JSON.stringify(a) !== JSON.stringify(b)) throw new Error(`${what}: ${JSON.stringify(a)} != ${JSON.stringify(b)}`);
};
const close = (a, b) => Math.abs(a - b) < 0.05;
const css = (node, prop) => {
    const [, r, g, b, a] = /^rgba?\((\d+), (\d+), (\d+)(?:, ([\d.]+))?\)$/.exec(getComputedStyle(node)[prop]);
    return [r, g, b, Math.round(255 * (a ?? 1))].map(x => Number(x).toString(16).padStart(2, '0')).join('');
};
for (const path of ['sample.txt', 'sample.js']) {
    const [theme, body, gutter, numbers, preset] = Array.from({length: 5}, () => new Compartment());
    let selection = fixedSelection;
    let expectedDoc = source;
    const view = new EditorView({parent: document.body, doc: source, selection,
        extensions: [EditorState.allowMultipleSelections.of(true), EditorView.lineWrapping,
            path.endsWith('.js') ? javascript() : [], theme.of(oneDarkTheme), body.of([]), gutter.of([]), numbers.of([]), preset.of([])]});
    view.dom.style.width = '228px';
    const configure = (compartment, value) => view.dispatch({effects: compartment.reconfigure(value)});
    const snapshot = (mode, labels, active, activeBody, themed = true) => {
        view.measure();
        equal(view.state.doc.toString(), expectedDoc, 'text');
        equal(view.state.selection.eq(selection), true, 'selection');
        const lines = [...view.contentDOM.querySelectorAll('.cm-line')];
        const container = view.dom.querySelector('.cm-gutters-before');
        const slot = container?.querySelector('.cm-lineNumbers');
        const bodyActive = lines.flatMap((n, i) => n.classList.contains('cm-activeLine') ? [i + 1] : []);
        equal(bodyActive, activeBody, `${mode} active body`);
        const record = {path, mode, focused: view.hasFocus, bodyActive, bodyFill: bodyActive.length ? css(lines[bodyActive[0] - 1], 'backgroundColor') : null,
            slots: [...(container?.children ?? [])].map(n => n.className), contentLeft: view.contentDOM.getBoundingClientRect().left - view.scrollDOM.getBoundingClientRect().left};
        if (!slot) {
            equal(labels, null, 'absent slot');
        } else {
            const [spacer, ...rows] = [...slot.children];
            const fg = themed ? '7d8799ff' : '6c6c6cff';
            const fill = themed ? '282c34ff' : 'f5f5f5ff';
            const highlight = themed ? '2c313aff' : 'e2f2ffff';
            equal(rows.map(n => n.textContent), labels, `${mode} labels`);
            equal(rows.length, view.state.doc.lines, 'one label per logical line');
            equal(css(container, 'color'), fg, 'gutter foreground');
            equal(css(container, 'backgroundColor'), fill, 'gutter fill');
            equal(getComputedStyle(container).borderRightWidth, themed ? '0px' : '1px', 'gutter border');
            equal(container.getAttribute('aria-hidden'), 'true', 'gutter accessibility');
            equal(getComputedStyle(spacer).visibility, 'hidden', 'spacer hidden');
            equal(getComputedStyle(container).position, 'sticky', 'gutter position');
            const slotRect = slot.getBoundingClientRect();
            record.width = slotRect.width;
            record.spacer = spacer.textContent;
            record.baseForeground = fg;
            record.baseBackground = fill;
            record.border = getComputedStyle(container).borderRightWidth;
            record.rows = rows.map((n, i) => {
                const box = n.getBoundingClientRect();
                const line = lines[i].getBoundingClientRect();
                const block = view.lineBlockAt(view.state.doc.line(i + 1).from);
                const marked = active.includes(i + 1);
                equal(n.classList.contains('cm-activeLineGutter'), marked, `${mode} active gutter line ${i + 1}`);
                equal(css(n, 'color'), fg, 'row foreground');
                equal(css(n, 'backgroundColor'), marked ? highlight : '00000000', 'row fill');
                equal(n.tabIndex, -1, 'not a tab stop');
                equal(close(line.top, box.top) && close(block.height, box.height) && close(slotRect.width, box.width), true, 'full row measurement');
                equal(document.elementFromPoint(box.left + 5, box.top + box.height / 2) === n, true, 'row hit');
                return {label: n.textContent, active: marked, color: css(n, 'color'), fill: css(n, 'backgroundColor'), height: box.height, blank: !lines[i].textContent};
            });
        }
        result.push(JSON.stringify(record));
        return record;
    };
    const defaults = ['1', '2', '3', '4'];
    view.focus();
    snapshot('theme-only', null, [], []);
    configure(gutter, highlightActiveLineGutter());
    snapshot('active-gutter-no-numbers', null, [], []);
    configure(gutter, []);
    configure(numbers, lineNumbers());
    const baseline = snapshot('numbers', defaults, [], []);
    configure(body, highlightActiveLine());
    snapshot('active-body-only', defaults, [], [1, 2]);
    configure(gutter, highlightActiveLineGutter());
    const focused = snapshot('both-focused', defaults, [1, 2], [1, 2]);
    blur.focus();
    const blurred = snapshot('both-blurred', defaults, [1, 2], [1, 2]);
    equal([focused.focused, blurred.focused], [true, false], 'focus');
    equal([focused.width, blurred.width], [baseline.width, baseline.width], 'focus and highlight width');
    configure(body, []);
    selection = EditorSelection.create([EditorSelection.cursor(source.length)]);
    view.dispatch({selection});
    snapshot('active-gutter-moved-to-trailing-blank', defaults, [4], []);
    configure(theme, []);
    snapshot('theme-removed', defaults, [4], [], false);
    configure(theme, oneDarkTheme);
    snapshot('theme-restored', defaults, [4], []);
    configure(numbers, lineNumbers({formatNumber: n => `Line ${String(n).padStart(2, '0')}`}));
    const formatted = snapshot('custom-labels', ['Line 01', 'Line 02', 'Line 03', 'Line 04'], [4], []);
    equal(formatted.width > baseline.width, true, 'formatted spacer width');
    configure(numbers, lineNumbers());
    equal(snapshot('decimal-restored', defaults, [4], []).width, baseline.width, 'restored width');
    configure(gutter, []);
    expectedDoc += 'a\nb\nc\nd\ne\nf\n';
    view.dispatch({changes: {from: source.length, insert: expectedDoc.slice(source.length)}, selection});
    const digitStep = snapshot('ten-lines', Array.from({length: 10}, (_, i) => String(i + 1)), [], []);
    equal(digitStep.spacer, '99', 'digit threshold');
    configure(numbers, []);
    snapshot('numbers-removed', null, [], []);
    configure(preset, basicSetup);
    snapshot('browser-basic-setup-only', Array.from({length: 10}, (_, i) => String(i + 1)), [4], [4]);
    view.destroy();
}
globalThis.nativeSupport = result;
