// Regenerate with Node 22.14+ and the pinned npm archive directory:
// node crates/codemirror/tests/fixtures/markdown_install.mjs <archive-root>
// node crates/codemirror/tests/fixtures/markdown_install.mjs <archive-root> --parser > crates/codemirror/tests/fixtures/markdown_parser.jsonl
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {pinnedModules} from './pinned_modules.mjs';

const root = realpathSync(mkdtempSync(join(tmpdir(), 'markdown-install-')));
const {pins, load} = pinnedModules(resolve(process.argv[2]), root);

const emit = value => console.log(JSON.stringify(value));
try {
    await (process.argv.includes('--parser') ? parserTrace() : installTrace());
} finally {
    rmSync(root, {recursive: true, force: true});
}

async function installTrace() {
    const packageName = '@codemirror/lang-markdown';
    const {markdown, pasteURLAsLink} = await load(packageName);
    const {Compartment, EditorState, Transaction} = await load('@codemirror/state');
    const {keymap, runScopeHandlers} = await load('@codemirror/view');
    const {foldable, getIndentation} = await load('@codemirror/language');
    const {CompletionContext} = await load('@codemirror/autocomplete');
    const {name, version, tarball, integrity, commit} = pins.get(packageName);
    const capture = 'markdown(config): installed keymap, fold service, language data, and paste handler';
    emit({capture, name, version, tarball, integrity, commit});

    // Use a headless dispatch harness. Read the plugin facet used by this pinned
    // ViewPlugin to run only the paste handler installed in the tested state.
    const pastePlugins = pasteURLAsLink.extension.at(-1).facet;
    const modes = [
        ['default', {}], ['no-keymap', {addKeymap: false}],
        ['no-tag-source', {completeHTMLTags: false}], ['no-paste', {pasteURLAsLink: false}],
        ['all-off', {addKeymap: false, completeHTMLTags: false, pasteURLAsLink: false}],
        ['removed', null], ['restored', {}],
    ];
    const clipboard = 'www.example.test/猫';
    const source = '# Top 😀\r\n\r\n- bullet\n\n## Child\r\n\r\nSelected 猫 and <sp\r\n\r\n# Next\r\n\r\n<div cl></div>\r\n';
    const fallback = keymap.of(['Enter', 'Backspace'].map(key => ({key, run(view) {
        view.fellThrough = true;
        return true;
    }})));
    const compartment = new Compartment();
    let state = EditorState.create({doc: source, extensions: [fallback, compartment.of(markdown())]});
    const unitAt = position => source.slice(0, position).replace(/\r\n/g, '\n').length;
    const rawAt = (text, position) => {
        let index = 0;
        for (let i = 0; i < position; i++) index += text.startsWith('\r\n', index) ? 2 : 1;
        return index;
    };
    const byteAt = position => Buffer.byteLength(source.slice(0, position));
    const byteFromUnit = position => byteAt(rawAt(source, position));
    const locate = token => {
        const at = source.indexOf(token);
        assert.ok(at >= 0, token);
        return at;
    };
    function dispatch(anchor, head, run) {
        let update;
        const view = {state: state.update({selection: {anchor: unitAt(anchor), head: unitAt(head)}}).state,
            dispatch(tr) { update = tr instanceof Transaction ? tr : this.state.update(tr); }, fellThrough: false};
        const handled = run(view);
        if (!update) return {handled, fallback: view.fellThrough};
        let output = source;
        const changes = [];
        update.changes.iterChanges((from, to, _fromB, _toB, insert) =>
            changes.unshift([rawAt(source, from), rawAt(source, to), insert.toString()]));
        for (const [from, to, insert] of changes) output = output.slice(0, from) + insert + output.slice(to);
        const selection = [update.newSelection.main.anchor, update.newSelection.main.head]
            .map(pos => Buffer.byteLength(output.slice(0, rawAt(output, pos))));
        return {handled, fallback: false, output, selection,
            event: update.annotation(Transaction.userEvent), scroll: update.scrollIntoView};
    }

    for (const [mode, config] of modes) {
        if (mode !== 'default') state = state.update({effects: compartment.reconfigure(config ? markdown(config) : [])}).state;
        const keys = [['Enter', locate('bullet') + 'bullet'.length], ['Backspace', locate('bullet')],
            ['Enter', locate('Selected') + 'Selected'.length]].map(([key, at]) => ({key, at: byteAt(at),
            ...dispatch(at, at, view => runScopeHandlers(view, {key}, 'editor'))}));
        const folds = ['# Top', '## Child', '# Next'].map(token => {
            const line = state.doc.lineAt(unitAt(locate(token)));
            const range = foldable(state, line.from, line.to);
            return {from: byteFromUnit(line.from), to: byteFromUnit(line.to),
                range: range ? [byteFromUnit(range.from), byteFromUnit(range.to)] : null,
                indent: getIndentation(state, line.from)};
        });
        const completions = ['<sp', '<div cl'].map(token => {
            const at = locate(token) + token.length;
            const context = new CompletionContext(state, unitAt(at), false);
            const sources = state.languageDataAt('autocomplete', context.pos);
            const results = sources.map(source => source(context)).filter(Boolean);
            return {at: byteAt(at), count: sources.length, results: results.map(result => ({
                range: [byteFromUnit(result.from), byteFromUnit(result.to ?? context.pos)],
                labels: result.options.map(option => option.label).filter(label => ['<span', '<div', 'class'].includes(label)),
            }))};
        });
        const from = locate('Selected');
        const to = from + 'Selected 猫'.length;
        const paste = {selection: [byteAt(to), byteAt(from)], expected: dispatch(to, from, view =>
            view.state.facet(pastePlugins).some(({plugin}) => plugin === pasteURLAsLink &&
                plugin.domEventHandlers.paste({clipboardData: {getData: type => type === 'text/plain' ? clipboard : ''}}, view)))};
        emit({mode, config, source, clipboard, keys, folds, completions, paste});
    }
}

async function parserTrace() {
    const {markdown, commonmarkLanguage, markdownLanguage} = await load('@codemirror/lang-markdown');
    const {json, jsonLanguage} = await load('@codemirror/lang-json');
    const {css, cssLanguage} = await load('@codemirror/lang-css');
    const {htmlLanguage} = await load('@codemirror/lang-html');
    const {Compartment, EditorState} = await load('@codemirror/state');
    const {forceParsing, LanguageDescription, syntaxTree} = await load('@codemirror/language');
    const {CompletionContext} = await load('@codemirror/autocomplete');
    const configured = [
        LanguageDescription.of({name: 'Data JSON', alias: ['data'], support: json()}),
        LanguageDescription.of({name: 'Style CSS', alias: ['style'], support: css()}),
    ];
    const catalog = {base: markdownLanguage, codeLanguages: configured, defaultCodeLanguage: cssLanguage};
    const resolver = {
        base: commonmarkLanguage, defaultCodeLanguage: jsonLanguage,
        codeLanguages(info) { return ['DATA', 'style', ''].includes(info) ? cssLanguage : null; },
    };
    let source = '# Heading 😀\r\n\r\n- [x] task\r\n\r\n```DATA attrs\r\n{"名":1}\r\n```\r\n\r\n' +
        '```my-data\r\n{"fuzzy":2}\r\n```\r\n\r\n```\r\na { --tone: red; color: var(--to) }\r\n```\r\n\r\n' +
        '```missing key\r\nb { --tone: red; color: var(--to) }\r\n```\r\n\r\n    c { --tone: red; color: var(--to) }\r\n\r\n' +
        '> ```style meta\r\n> d { --tone: red; color: var(--to) }\r\n> ```\r\n\r\n```\uFEFFmask\r\ne { --tone: red; color: var(--to) }\r\n```\r\n\r\n<div cl></div>\r\n';
    const compartment = new Compartment();
    let state = EditorState.create({doc: source, extensions: compartment.of(markdown())});
    const bytes = pos => Buffer.byteLength(source.slice(0, pos));
    const units = pos => source.slice(0, pos).replace(/\r\n/g, '\n').length;
    for (const [mode, config] of [['stock', null], ['catalog', catalog], ['resolver', resolver],
        ['edit-info', catalog], ['restore-stock', {}]]) {
        let change;
        if (config) {
            const transaction = {effects: compartment.reconfigure(markdown(config))};
            if (mode === 'edit-info') {
                const at = source.indexOf('DATA');
                transaction.changes = {from: units(at), to: units(at + 4), insert: 'style'};
                change = [bytes(at), bytes(at + 4), 'style'];
                source = source.slice(0, at) + 'style' + source.slice(at + 4);
            }
            state = state.update(transaction).state;
        }
        const view = {state, dispatch(spec) { this.state = this.state.update(spec).state; }};
        assert.ok(forceParsing(view, state.doc.length, 5000));
        state = view.state;
        const variables = [...'abcde'].map(name => `${name} { --tone: red; color: var(--to`);
        const probes = ['Heading 😀', '{"名', '{"fuzzy', ...variables, '<div cl'].map(token => {
            const at = source.indexOf(token) + token.length;
            const pos = units(at);
            const language = [jsonLanguage, cssLanguage, htmlLanguage]
                .find(lang => lang.isActiveAt(state, pos))?.name ?? 'markdown';
            const found = state.languageDataAt('commentTokens', pos)[0];
            const context = new CompletionContext(state, pos, false);
            const variable = state.languageDataAt('autocomplete', pos).some(complete =>
                complete(context)?.options.some(option => option.label === '--tone'));
            return {at: bytes(at), language, comment: found?.block && [found.block.open, found.block.close], variable};
        });
        const task = syntaxTree(state).toString().includes('TaskMarker');
        emit({mode, source, change, task, probes});
    }
}
