// Run with Node and a directory of local SHA-512-pinned npm archives.
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync, symlinkSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {projectedOffsets} from './editor_line_offsets.mjs';
import {pinnedModules} from './pinned_modules.mjs';

assert.equal(process.argv.length, 3, 'provide the directory of local npm archives');
const root = mkdtempSync(join(realpathSync(tmpdir()), 'lr-fold-props.'));
const path = process.env.PATH;
try {
    symlinkSync('/usr/bin/tar', join(root, 'tar'));
    process.env.PATH = root;
    const {load} = pinnedModules(resolve(process.argv[2]), root);
    const {jsonLanguage} = await load('@codemirror/lang-json');
    const {lezerLanguage} = await load('@codemirror/lang-lezer');
    const {markdown} = await load('@codemirror/lang-markdown');
    const {Compartment, EditorState} = await load('@codemirror/state');
    const {foldable, foldNodeProp, foldService, syntaxTree} = await load('@codemirror/language');
    const span = (node, byteAt) => node ? [node.name, byteAt(node.from), byteAt(node.to)] : null;
    for (const [name, stock, body, editFrom, targets] of [
        ['json', jsonLanguage, '[{ \r\n  "😀": [\r 0\r\n ]\r\n}]', '0', ['Array', 'Object']],
        ['lezer', lezerLanguage, '@top Main { \r\n word "😀"\r  "x"\r\n}', 'word', ['Body']],
    ]) {
        for (const scope of ['standalone', 'ordinary', 'quoted']) {
            const prefix = scope === 'quoted' ? '> ' : '';
            const source = scope === 'standalone' ? body : '前 😀\r\n\r\n' + [
                `\`\`\`${name}`, ...body.split(/\r\n|\r/), '```',
            ].map(line => prefix + line).join('\r\n');
            const origin = scope === 'standalone' ? 0 : source.indexOf(body.split('\r')[0]);
            const calls = [];
            let project = projectedOffsets(source).byteAt;
            const record = (node, state) => {
                const children = [];
                for (let child = node.firstChild; child; child = child.nextSibling) {
                    children.push(span(child, project));
                }
                calls.push([span(node, project), span(node.parent, project), children,
                    project(state.selection.main.head)]);
                const from = node.firstChild.to + (state.selection.main.head === 0 ? 0 : 1);
                return {from, to: node.lastChild.from};
            };
            const configure = callback => stock.configure({props: [
                foldNodeProp.add(type => targets.includes(type.name) ? callback : undefined),
            ]});
            const install = parser => scope === 'standalone' ? parser :
                markdown({codeLanguages: info => info === name ? parser : null});
            const slot = new Compartment();
            const create = (...extensions) => EditorState.create({doc: source, extensions});
            const state = create(slot.of(install(configure(record))));
            const from = state.doc.toString().indexOf(body.split('\r')[0]);
            const line = state.doc.lineAt(from);
            const head = state.doc.toString().indexOf(editFrom, from);
            assert.equal(project(from), Buffer.byteLength(source.slice(0, origin)));
            const checks = [];
            const query = (step, current, input, start = line.from, end = line.to) => {
                project = projectedOffsets(input).byteAt;
                const fold = foldable(current, start, end);
                checks.push({step, line: [project(start), project(end)], range: fold ?
                    [project(fold.from), project(fold.to)] : null, calls: calls.splice(0)});
            };
            const first = create(install(stock));
            const selection = state.update({selection: {anchor: head}}).state;
            assert.equal(syntaxTree(state), syntaxTree(selection));
            query('stock', first, source);
            query('configured', state, source);
            query('selected', selection, source);
            const disabled = selection.update({effects: slot.reconfigure(install(configure(null)))}).state;
            assert.notEqual(syntaxTree(disabled), syntaxTree(selection));
            query('disabled', disabled, source);
            query('retained', selection, source);
            query('service', create(install(configure(record)), foldService.of(() => ({
                from: line.to, to: line.to + 1,
            }))), source);
            if (scope === 'quoted') query('gap', state, source, line.from, line.to + 2);
            const byteAt = projectedOffsets(source).byteAt;
            const at = byteAt(head);
            const editedSource = Buffer.concat([
                Buffer.from(source).subarray(0, at), Buffer.from('0'), Buffer.from(source).subarray(at),
            ]).toString();
            const edited = selection.update({changes: {from: head, insert: '0'}}).state;
            query('edited', edited, editedSource);
            console.log(JSON.stringify({name, scope, source, targets, head: at, checks}));
        }
    }
} finally {
    process.env.PATH = path;
    rmSync(root, {recursive: true, force: true});
}
