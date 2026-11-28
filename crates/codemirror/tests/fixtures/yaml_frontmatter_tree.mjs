// Capture with Node and the directory of SHA-512-pinned npm archives.
import assert from 'node:assert/strict';
import {mkdtempSync, readFileSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {projectedOffsets} from './editor_line_offsets.mjs';
import {pinnedModules} from './pinned_modules.mjs';

const scratch = mkdtempSync(join(realpathSync(tmpdir()), 'yaml-frontmatter-tree.'));
try {
    const {load, install, pins} = pinnedModules(resolve(process.argv[2]), scratch);
    const name = '@codemirror/lang-yaml';
    assert.equal(pins.get(name).version, '6.1.3');
    const packaged = readFileSync(install(name).url, 'utf8');
    const native = readFileSync(new URL('../../src/language/yaml/frontmatter_language/tree.rs', import.meta.url), 'utf8');
    for (const [from, to] of [['states', 'states'], ['stateData', 'state_data'], ['goto', 'goto'], ['tokenData', 'token_data']]) {
        const field = (source, key) => source.match(new RegExp(`\\b${key}: "([^"]*)"`))?.[1];
        assert.ok(field(packaged, from));
        assert.equal(field(native, to), field(packaged, from), to);
    }
    const {yamlFrontmatter} = await load('@codemirror/lang-yaml');
    const {jsonLanguage} = await load('@codemirror/lang-json');
    const {EditorState} = await load('@codemirror/state');
    const {ensureSyntaxTree} = await load('@codemirror/language');
    const {IterMode, NodeProp} = await load('@lezer/common');
    const language = yamlFrontmatter({content: jsonLanguage});
    const cases = [
        '', '{}', '\r\n{}', '😀\r23', '---', '---\n', '---\r', '---\r\n',
        '---\n---', '---\n---\n', '---\n\n---\n{}', '---\r\n---\r\n{}',
        '---\r\ntitle: 😀\r---\r{}', '---\rtitle: 前\n---\r\n{}',
        '---\ntitle: 😀\n', '---\n\nwhere: here', '--- \n---\n[]',
        ' ---\n---\n[]', '----\n---\n[]', '---\n ---\n{}',
        '---\n--- \n{}', '---\nx: 1\n---\n---\n[]', '---\n---\n😀',
        '\uffff', '\uffff\n{}', '---\n\uffff\n---\n{}', '---\nx: \uffff\n---\n{}',
        '---\n---\n\uffff', '---\n---\n\uffff\r\n{}', '😀\r\uffff\n{}',
        '\ufeff---\n---\n[]', '\u2028---\n---\n[]', '---\u2028---\n[]',
    ];
    for (const source of cases) {
        const state = EditorState.create({doc: source, extensions: language});
        const {byteAt, units} = projectedOffsets(source);
        const tree = ensureSyntaxTree(state, units, 10000);
        assert(tree && tree.length === units);
        let depth = 0;
        const nodes = [], mounts = [];
        tree.iterate({mode: IterMode.IgnoreMounts,
            enter(node) {
                const from = byteAt(node.from), to = byteAt(node.to);
                nodes.push([depth++, node.name, from, to]);
                const mounted = node.tree?.prop(NodeProp.mounted);
                if (mounted) mounts.push([node.name, from, to, mounted.tree.type.name]);
            },
            leave() { depth--; },
        });
        console.log(JSON.stringify({source, nodes, mounts}));
    }
} finally {
    rmSync(scratch, {recursive: true, force: true});
}
