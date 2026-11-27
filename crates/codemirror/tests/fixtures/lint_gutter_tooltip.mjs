// Run with capture_browser_fixture.mjs, package @codemirror/lint and the verified archives.
import {lintGutter, setDiagnostics} from '@codemirror/lint';
import {EditorState} from '@codemirror/state';
import {EditorView} from '@codemirror/view';

const source = 'é abc\r\nxy';
const specs = [
    {from: 2, to: 5, severity: 'error', message: 'first'},
    {from: 3, to: 4, severity: 'warning', message: 'second'},
    {from: 6, to: 8, severity: 'hint', message: 'tail'},
    {from: 6, to: 6, severity: 'error', message: 'skip'},
];
const rows = [];
for (const name of ['default', 'filtered', 'empty', 'conflict']) {
    const markerFilter = diagnostics => diagnostics.filter(d => d.message !== 'skip');
    const visible = name === 'filtered' ? diagnostics => diagnostics.filter(d => d.message !== 'first')
        : name === 'empty' ? () => [] : undefined;
    const config = lintGutter({markerFilter, tooltipFilter: visible,
        hoverTime: name === 'filtered' ? 45 : undefined});
    let editor;
    try {
        const extensions = name === 'conflict' ? [config, lintGutter({hoverTime: 100}), lintGutter({hoverTime: 200})] : config;
        editor = new EditorView({parent: document.body, state: EditorState.create({doc: source, extensions})});
        editor.dispatch(setDiagnostics(editor.state, specs));
        editor.measure();
        const marks = [...editor.dom.querySelectorAll('.cm-lint-marker')].map((marker, index) => {
            const actualTimeout = window.setTimeout;
            let delay = null, fire;
            try {
                window.setTimeout = (call, time) => {
                    delay = time;
                    fire = call;
                    return 0;
                };
                marker.onmouseover?.();
            } finally {
                window.setTimeout = actualTimeout;
            }
            fire?.();
            editor.measure();
            const messages = [...editor.dom.querySelectorAll('.cm-tooltip-lint .cm-diagnosticText')]
                .map(node => node.textContent);
            return {line: index === 0 ? 0 : 8, severity: marker.className.split('cm-lint-marker-').at(-1), delay, messages};
        });
        rows.push({source, name, marks});
    } catch (error) {
        if (!String(error).includes('hoverTime')) throw error;
        rows.push({source, name, conflict: true});
    } finally {
        editor?.destroy();
    }
}
globalThis.nativeSupport = rows.map(row => JSON.stringify(row));
