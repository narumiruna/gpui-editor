// node crates/codemirror/tests/fixtures/yaml_frontmatter_indent.mjs <archive-root>
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {projectedOffsets} from './editor_line_offsets.mjs';
import {pinnedModules} from './pinned_modules.mjs';
const root = realpathSync(mkdtempSync(join(tmpdir(), 'yaml-frontmatter-indent-')));
try {
    const {load} = pinnedModules(resolve(process.argv[2]), root);
    const {yamlFrontmatter} = await load('@codemirror/lang-yaml');
    const {jsonLanguage} = await load('@codemirror/lang-json');
    const {lezerLanguage} = await load('@codemirror/lang-lezer');
    const {EditorState} = await load('@codemirror/state');
    const {getIndentation, IndentContext, indentUnit} = await load('@codemirror/language');
    for (const [name, content, source] of [
        ['json', jsonLanguage, '---\r\nfoo:\r  - [a, b,\r\n     c]\r\n---\r\n{\r\n  "😀": [\r  1\r\n  ]\r\n}'],
        ['json', jsonLanguage, '---\r\nfoo: [x,\r\n  y\r\n---\r\n'],
        ['json', jsonLanguage, '{\r\n  "未": {\r\n "x": 0\r\n  }\r\n}'],
        ['lezer', lezerLanguage, '---\r\nfoo: |\r\n  line\r\n---\r\n@top Main {\r\n   "😀" |\r "x"\r\n}'],
        ['lezer', lezerLanguage, '---\r\nfoo: [x,\r\n  y'],
        ['json', jsonLanguage, '---\r\nfoo: "quoted 😀\r\n  string"\r\nbar: value\r\n---\r\n[\r\n  1,\r\n'],
        ['lezer', lezerLanguage, '---\r\n---\r\n'],
    ]) {
        const state = EditorState.create({doc: source, extensions: [yamlFrontmatter({content}), indentUnit.of('   ')]});
        const {byteAt, units} = projectedOffsets(source);
        assert.equal(state.doc.length, units);
        const normalized = state.doc.toString();
        const queries = [];
        for (let pos = 0; pos <= units;) {
            const line = state.doc.lineAt(pos);
            const sim = new IndentContext(state, {simulateBreak: pos, simulateDoubleBreak: true,
                overrideIndentation: at => at === line.from ? 7 : -1});
            queries.push([byteAt(pos), getIndentation(state, pos), getIndentation(sim, pos)]);
            if (pos === units) break;
            pos += normalized.codePointAt(pos) > 0xffff ? 2 : 1;
        }
        console.log(JSON.stringify({name, source, queries}));
    }
} finally {
    rmSync(root, {recursive: true, force: true});
}
