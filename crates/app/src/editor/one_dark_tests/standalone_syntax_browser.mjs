// Captured in Chrome 158.0.8127.37 with the SHA-512-pinned packages.
// Replay with capture_browser_fixture.mjs and @codemirror/theme-one-dark,@codemirror/lang-javascript.
import {javascript} from '@codemirror/lang-javascript';
import {syntaxHighlighting} from '@codemirror/language';
import {Compartment} from '@codemirror/state';
import {oneDark, oneDarkHighlightStyle, oneDarkTheme} from '@codemirror/theme-one-dark';
import {EditorView} from '@codemirror/view';

const source = "function greet() {\n  const label = '茶🐱'; // café\n  return 42;\n}\ngreet();\n";
const config = new Compartment();
const view = new EditorView({doc: source, parent: document.body, extensions: [javascript(), config.of([])]});
const samples = [
    ['keyword', 'return'],
    ['string', "'茶🐱'"],
    ['number', '42'],
    ['function', 'greet'],
    ['comment', '// café'],
    ['untagged', '  '],
];
const bytes = text => new TextEncoder().encode(text).length;
function color(css) {
    const match = /^rgba?\((\d+), (\d+), (\d+)(?:, ([\d.]+))?\)$/.exec(css);
    if (!match) throw new Error(`Not an RGB color: ${css}`);
    return [...match.slice(1, 4).map(Number), Math.round(Number(match[4] ?? 1) * 255)]
        .map(part => part.toString(16).padStart(2, '0')).join('');
}

function sample(role, text) {
    const start = role === 'function' ? source.lastIndexOf(text) : source.indexOf(text);
    const end = start + text.length;
    let lineStart = 0;
    for (const line of view.contentDOM.querySelectorAll('.cm-line')) {
        const lineEnd = lineStart + line.textContent.length;
        if (lineStart <= start && end <= lineEnd) {
            const colors = new Set();
            const styled = new Set();
            const walker = document.createTreeWalker(line, NodeFilter.SHOW_TEXT);
            let at = lineStart;
            for (let node; (node = walker.nextNode());) {
                const next = at + node.length;
                if (at < end && start < next) {
                    colors.add(color(getComputedStyle(node.parentElement).color));
                    styled.add(node.parentElement !== line);
                }
                at = next;
            }
            if (colors.size !== 1 || styled.size !== 1) throw new Error(`Mixed or missing paint: ${role}`);
            return {role, text, range: [bytes(source.slice(0, start)), bytes(source.slice(0, end))],
                color: [...colors][0], styled: [...styled][0]};
        }
        lineStart = lineEnd + 1;
    }
    throw new Error(`Missing sample: ${role}`);
}

const records = [];
for (const [mode, extensions] of [
    ['initial', []],
    ['view-only', oneDarkTheme],
    ['highlight-only', syntaxHighlighting(oneDarkHighlightStyle)],
    ['combined', oneDark],
    ['removed', []],
]) {
    view.dispatch({effects: config.reconfigure(extensions)});
    if (view.state.doc.toString() !== source) throw new Error('Reconfiguration changed the source');
    const {color: foreground, backgroundColor} = getComputedStyle(view.dom);
    records.push(JSON.stringify({mode, source, foreground: color(foreground),
        surface: color(backgroundColor), samples: samples.map(args => sample(...args))}));
}
view.destroy();
globalThis.nativeSupport = records;
