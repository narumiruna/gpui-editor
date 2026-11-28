// Captured in Chrome 158.0.8127.37 with the SHA-512-pinned packages.
// Replay with capture_browser_fixture.mjs and @codemirror/lang-markdown,@codemirror/theme-one-dark.
import {html} from '@codemirror/lang-html';
import {markdown} from '@codemirror/lang-markdown';
import {oneDark, oneDarkTheme} from '@codemirror/theme-one-dark';
import {EditorView} from '@codemirror/view';

const source = '<p title="字🐱">茶</p>\n\n<style>\n\ta { content: "字🐱"; color: red; }\n</style>\n\n<script>\n\tconst n = 42;\n</script>\n';
const samples = [
    [0, 'title'], [0, '"字🐱"'], [0, '茶'], [2, 'style'],
    [3, 'content'], [3, '"字🐱"'], [3, '; '], [3, 'color'],
    [6, 'script'], [7, 'const'], [7, '42'], [8, 'script'],
];

function rgb(css) {
    const match = /^rgb\((\d+), (\d+), (\d+)\)$/.exec(css);
    if (!match) throw new Error(`Not an RGB color: ${css}`);
    return match.slice(1).map(value => Number(value).toString(16).padStart(2, '0')).join('');
}

function sample(line, token, row) {
    const start = line.textContent.indexOf(token);
    if (start < 0) throw new Error(`Missing ${token} on line ${row}`);
    const end = start + token.length;
    const walker = document.createTreeWalker(line, NodeFilter.SHOW_TEXT);
    const colors = new Set();
    let at = 0;
    for (let node; (node = walker.nextNode());) {
        const next = at + node.length;
        if (at < end && start < next) colors.add(rgb(getComputedStyle(node.parentElement).color));
        at = next;
    }
    if (colors.size !== 1) throw new Error(`Mixed or missing color for ${token} on line ${row}`);
    return {row, token, color: [...colors][0]};
}

const output = [];
for (const [language, extension] of [['html', html()], ['markdown', markdown()]]) {
    for (const [mode, theme] of [['view-only', oneDarkTheme], ['combined', oneDark]]) {
        const view = new EditorView({doc: source, extensions: [extension, theme], parent: document.body});
        view.measure();
        const lines = view.contentDOM.querySelectorAll('.cm-line');
        if (Array.from(lines, line => line.textContent).join('\n') !== source) {
            throw new Error('The browser did not display all source lines');
        }
        output.push(JSON.stringify({language, mode, source,
            samples: samples.map(([row, token]) => sample(lines[row], token, row))}));
        view.destroy();
    }
}
globalThis.nativeSupport = output;
