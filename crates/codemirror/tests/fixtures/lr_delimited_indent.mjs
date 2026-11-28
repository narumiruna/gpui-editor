// Run with Node and a directory of local SHA-512-pinned npm archives.
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync, symlinkSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {projectedOffsets} from './editor_line_offsets.mjs';
import {pinnedModules} from './pinned_modules.mjs';

assert.equal(process.argv.length, 3, 'provide the directory of local npm archives');
const root = mkdtempSync(join(realpathSync(tmpdir()), 'lr-delimited-indent.'));
const path = process.env.PATH;
try {
    symlinkSync('/usr/bin/tar', join(root, 'tar'));
    process.env.PATH = root; // Do not let the loader fall back to curl.
    const {load} = pinnedModules(resolve(process.argv[2]), root);
    const {jsonLanguage} = await load('@codemirror/lang-json');
    const {lezerLanguage} = await load('@codemirror/lang-lezer');
    const {markdown} = await load('@codemirror/lang-markdown');
    const {wastLanguage} = await load('@codemirror/lang-wast');
    const {EditorState} = await load('@codemirror/state');
    const {getIndentation, IndentContext, indentNodeProp} = await load('@codemirror/language');
    const languages = {json: jsonLanguage, lezer: lezerLanguage, wast: wastLanguage};
    const quote = body => `前 😀\r\n\r\n> \`\`\`lezer\r\n> ${body.replaceAll('\r\n', '\r\n> ')}\r\n> \`\`\`\r\n`;
    const projected = quote('// 前 😀\r\nR{¦}');
    for (const [name, withPosition, grammar = 'lezer', mode = 'stock', mounted = false] of [
        ['empty body', 'R{¦}'],
        ['space before closer', 'R{¦  }'],
        ['empty inner parentheses', 'R{(¦)}'],
        ['nonempty body at closer', 'R{a¦}'],
        ['nonempty body at opener end', 'R{¦  a}'],
        ['aligned body on the next line', 'Rule{  a\r\n¦  }'],
        ['empty body on the next line', 'R{\r\n¦}'],
        ['nonempty, not aligned', '    R{\n  a\n¦}'],
        ['skipped comment before same-line content', 'R{ /*x*/¦a\r\n}'],
        ['skipped comment before same-line closer', 'R{ /*x*/ ¦}'],
        ['skipped comment on its own line', 'R{ //x\r\n¦}'],
        ['query before a skipped comment', 'R{¦ /*x*/ }'],
        ['skipped recovery before closer', 'R{?¦}'],
        ['missing closer without content', 'R{¦'],
        ['missing closer with aligned content', 'Rule{  a¦'],
        ['missing closer with skipped comment', 'Rule{ /*x*/¦'],
        ['override Body', 'R{¦}', 'lezer', 'override'],
        ['remove Body override', 'R{¦}', 'lezer', 'remove'],
        ['standalone projected closer', '// 前 😀\r\n/*猫😀*/ R{¦}'],
        ['quoted projected closer', projected, 'lezer', 'stock', true],
        ['quoted projected next-line closer', quote('R{\r\n  ¦}'), 'lezer', 'stock', true],
        ['quoted projected Body range', projected, 'lezer', 'override', true],
        ['quoted projected Body override removed', projected, 'lezer', 'remove', true],
        ['stock JSON object', '{¦}', 'json'],
        ['stock JSON array under an object', '{"😀":[¦]}', 'json'],
        ['JSON array with stock rule removed', '{"😀":[¦]}', 'json', 'remove'],
        ['stock WAST App with matching closer', '(module (¦))', 'wast'],
        ['stock WAST App with nonempty body', '(module (; note ;) (func¦))', 'wast'],
    ]) {
        const at = withPosition.slice(0, withPosition.indexOf('¦')).replace(/\r\n?/g, '\n').length;
        const source = withPosition.replace('¦', '');
        const {byteAt} = projectedOffsets(source);
        const calls = [];
        const span = node => [node.name, byteAt(node.from), byteAt(node.to)];
        const target = grammar === 'json' ? 'Array' : 'Body';
        const stock = languages[grammar];
        let language = stock;
        if (mode === 'override' || (mode === 'remove' && grammar === 'lezer')) {
            language = language.configure({props: [indentNodeProp.add({[target]: cx => {
                const {node} = cx;
                calls.push([span(node), span(node.firstChild), span(node.lastChild), byteAt(cx.pos)]);
                return 7;
            }})]});
        }
        if (mode === 'remove') {
            language = language.configure({props: [indentNodeProp.add(type => type.name === target ? null : undefined)]});
        }
        const state = EditorState.create({doc: source, extensions: mounted
            ? markdown({codeLanguages: info => info === 'lezer' ? language : null}) : language});
        const columns = [{}, {simulateBreak: at}, {simulateBreak: at, simulateDoubleBreak: true}]
            .map(options => getIndentation(new IndentContext(state, options), at));
        console.log(JSON.stringify({name, grammar, mode, mounted, source, at: byteAt(at), columns, calls}));
    }
} finally {
    process.env.PATH = path;
    rmSync(root, {recursive: true, force: true});
}
