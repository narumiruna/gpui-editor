// Replay with capture_browser_fixture.mjs and @codemirror/merge,@codemirror/lang-markdown,@codemirror/theme-one-dark.
// Captured in Chrome 158.0.8127.37 with the SHA-512-pinned packages.
import {commonmarkLanguage, markdown} from '@codemirror/lang-markdown';
import {HighlightStyle, syntaxHighlighting} from '@codemirror/language';
import {tags} from '@lezer/highlight';
import {MergeView} from '@codemirror/merge';
import {Compartment} from '@codemirror/state';
import {oneDark, oneDarkHighlightStyle, oneDarkTheme} from '@codemirror/theme-one-dark';

const source = prefix => [
    '~~~md opts', '# Heading',
    `${prefix} **Bold** *Italic* [Link](https://example.invalid) ~~Strike~~`,
    '~~~', '', '~~~unknown', '**Still plain**', '~~~',
].join('\r\n');
const sources = [source('字🐱'), source('茶')];
const config = new Compartment();
const setup = doc => ({doc, extensions: [
    markdown({codeLanguages: info => info.split(/\s/)[0] === 'md' ? commonmarkLanguage : null}),
    config.of([]),
]});
const merge = new MergeView({a: setup(sources[0]), b: setup(sources[1]), parent: document.body});
const bytes = text => new TextEncoder().encode(text).length;
function rgb(css) {
    const match = /^rgb\((\d+), (\d+), (\d+)\)$/.exec(css);
    if (!match) throw Error(`Not an RGB color: ${css}`);
    return (Number(match[1]) << 16) | (Number(match[2]) << 8) | Number(match[3]);
}

// Native Markdown fences use CommonMark, which does not tag ~~Strike~~.
// Apply line-through to a recognized token to check the decoration on its own.
const strike = syntaxHighlighting(HighlightStyle.define([{tag: tags.strong, textDecoration: 'line-through'}]));
const output = [];
for (const [mode, extensions] of [
    ['view-only', oneDarkTheme], ['combined', oneDark],
    ['highlight-only', syntaxHighlighting(oneDarkHighlightStyle)], ['strike-only', strike], ['removed', []],
]) {
    const sides = [merge.a, merge.b].map((view, side) => {
        view.dispatch({effects: config.reconfigure(extensions)});
        view.measure();
        const source = sources[side];
        if (view.state.doc.toString() !== source.replaceAll('\r\n', '\n')) throw Error('Source changed');
        const lines = [...view.contentDOM.querySelectorAll('.cm-line')];
        const samples = ['Heading', 'Bold', 'Italic', 'Link', 'Strike', 'Still plain'].map(text => {
            const line = lines.find(el => el.textContent.includes(text));
            if (!line) throw Error(`Missing line for ${text}`);
            const node = [...line.querySelectorAll('span')].reverse().find(el => el.textContent.includes(text)) ?? line;
            const from = line.textContent.indexOf(text);
            const {color, fontWeight, fontStyle, textDecorationLine} = getComputedStyle(node);
            return {text, line: lines.indexOf(line) + 1,
                range: [bytes(line.textContent.slice(0, from)), bytes(line.textContent.slice(0, from + text.length))],
                color: rgb(color), weight: Number(fontWeight), italic: fontStyle === 'italic', decoration: textDecorationLine};
        });
        return {source, foreground: rgb(getComputedStyle(view.contentDOM).color), samples};
    });
    output.push(JSON.stringify({mode, sides}));
}
merge.destroy();
globalThis.nativeSupport = output;
