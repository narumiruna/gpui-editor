// Run with Node and a directory of local SHA-512-pinned npm archives.
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync, symlinkSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {projectedOffsets} from './editor_line_offsets.mjs';
import {pinnedModules} from './pinned_modules.mjs';

assert.equal(process.argv.length, 3, 'provide the directory of local npm archives');
const root = mkdtempSync(join(realpathSync(tmpdir()), 'lr-indent-props.'));
const path = process.env.PATH;
try {
    symlinkSync('/usr/bin/tar', join(root, 'tar'));
    process.env.PATH = root;
    const {load} = pinnedModules(resolve(process.argv[2]), root);
    const {jsonLanguage} = await load('@codemirror/lang-json');
    const {lezerLanguage} = await load('@codemirror/lang-lezer');
    const {markdown} = await load('@codemirror/lang-markdown');
    const {yamlFrontmatter} = await load('@codemirror/lang-yaml');
    const {Compartment, EditorState} = await load('@codemirror/state');
    const {getIndentation, IndentContext, indentNodeProp, indentService, indentUnit, syntaxTree} =
        await load('@codemirror/language');
    const span = (node, byteAt) => node ? [node.name, byteAt(node.from), byteAt(node.to)] : null;
    for (const [name, stock, body, target, leaf, word] of [
        ['json', jsonLanguage, '\t[{\r\n  "😀": [\r   20\r\n  ]\r\n}]', 'Array', 'Number', '20'],
        ['lezer', lezerLanguage, '\t@top Main {\r\n  word "😀"\r    rule\r\n}', 'Body', 'Literal', 'word'],
    ]) {
        for (const scope of ['standalone', 'ordinary', 'quoted', 'yaml']) {
            const marker = scope === 'quoted' ? '> ' : '';
            const input = scope === 'standalone' ? body : scope === 'yaml' ? '---\r\ntitle: 前 😀\r---\r\n' + body : '前 😀\r\n\r\n' + marker +
                `\`\`\`${name}\r\n` + marker + body.replace(/\r\n|\r/g, br => br + marker) + '\r\n' +
                marker + '```\r\n\r\nTail.';
            const calls = [];
            const configure = (offset, strategy = true) => stock.configure({props: [indentNodeProp.add(type => {
                if (type.name === target) {
                    if (!strategy) return null;
                    return cx => {
                        const next = cx.continue();
                        calls.push([span(cx.node, byteAt), span(cx.node.parent, byteAt),
                            byteAt(cx.pos), byteAt(cx.state.selection.main.head), cx.baseIndent, cx.unit,
                            next, cx.textAfter]);
                        return offset + cx.baseIndent + cx.unit + (next ?? 0);
                    };
                }
                if (strategy && type.name === leaf) return () => null;
            })]});
            const install = parser => scope === 'standalone' ? parser : scope === 'yaml' ?
                yamlFrontmatter({content: parser}) :
                markdown({codeLanguages: info => info === name ? parser : null});
            const slot = new Compartment();
            const create = (...extensions) => EditorState.create({doc: input, extensions: [
                indentUnit.of('   '), EditorState.tabSize.of(8), ...extensions,
            ]});
            const original = create(slot.of(install(configure(7))));
            const normalized = original.doc.toString();
            const pos = normalized.indexOf(word);
            const insideLeaf = name === 'json' ? pos + 1 : normalized.indexOf('😀', normalized.indexOf('@top')) + 2;
            const closer = normalized.lastIndexOf(name === 'json' ? ']' : '}');
            const bodyLine = original.doc.lineAt(normalized.indexOf(name === 'json' ? '[{' : '@top')).from;
            const {byteAt: inputByte} = projectedOffsets(input);
            let byteAt = inputByte;
            const selected = original.update({selection: {anchor: pos + word.length}}).state;
            assert.equal(syntaxTree(selected), syntaxTree(original));
            const replacement = selected.update({effects: slot.reconfigure(install(configure(11)))}).state;
            assert.notEqual(syntaxTree(selected), syntaxTree(replacement));
            const disabled = selected.update({effects: slot.reconfigure(install(configure(0, false)))}).state;
            const checks = [];
            const query = (step, state, at, simulate = false) => {
                const context = new IndentContext(state, simulate ? {
                    simulateBreak: at, simulateDoubleBreak: true,
                    overrideIndentation: start => start === bodyLine ? 5 : -1,
                } : {});
                const column = getIndentation(context, at);
                checks.push({step, at: byteAt(at), column, calls: calls.splice(0)});
            };
            query('stock', create(install(stock)), pos);
            query('configured', original, pos);
            query('inside', selected, insideLeaf);
            query('selected', selected, pos);
            query('simulated', selected, closer, true);
            query('replacement', replacement, pos);
            query('retained', selected, pos);
            query('disabled', disabled, pos);
            query('service', create(install(configure(7)), indentService.of(() => 17)), pos);
            const edit = {from: pos, insert: name === 'json' ? '1' : 'x'};
            const editAt = inputByte(pos);
            const bytes = Buffer.from(input);
            const editedInput = Buffer.concat([bytes.subarray(0, editAt), Buffer.from(edit.insert), bytes.subarray(editAt)]).toString();
            byteAt = projectedOffsets(editedInput).byteAt;
            query('edited', selected.update({changes: edit}).state, pos + 1);
            console.log(JSON.stringify({name, scope, input, bodyLine: inputByte(bodyLine), target,
                leaf, head: inputByte(pos + word.length), editAt, insert: edit.insert, checks}));
        }
    }
} finally {
    process.env.PATH = path;
    rmSync(root, {recursive: true, force: true});
}
