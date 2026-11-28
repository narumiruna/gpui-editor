// Run with Node and the directory of SHA-512-pinned npm archives.
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {projectedOffsets} from './editor_line_offsets.mjs';
import {pinnedModules} from './pinned_modules.mjs';

const scratch = mkdtempSync(join(realpathSync(tmpdir()), 'gpui-yaml-frontmatter-wast.'));
try {
    const {load} = pinnedModules(resolve(process.argv[2]), scratch);
    const {EditorState, Compartment, Prec} = await load('@codemirror/state');
    const {defineLanguageFacet, foldable, foldNodeProp, foldService, indentNodeProp, languageDataProp, sublanguageProp,
        getIndentation, HighlightStyle, IndentContext, indentUnit, syntaxHighlighting, syntaxTree} = await load('@codemirror/language');
    const {tags, styleTags, highlightTree} = await load('@lezer/highlight');
    const {NodeProp, IterMode} = await load('@lezer/common');
    const {yamlFrontmatter} = await load('@codemirror/lang-yaml');
    const {wastLanguage} = await load('@codemirror/lang-wast');
    const root = defineLanguageFacet({probe: 'root'});
    const extended = defineLanguageFacet({probe: 'extra'});
    const replaced = defineLanguageFacet({probe: 'replace'});
    const identity = new NodeProp();
    const dataCalls = [], indentCalls = [], foldCalls = [];
    let byteAt;
    const span = node => [node.name, byteAt(node.from), byteAt(node.to)];
    const configured = (name, strict, tag) => {
        const language = wastLanguage.configure({strict, props: [
            languageDataProp.add(type => type.isTop ? root : undefined),
            sublanguageProp.add({Module: [
                {type: 'extend', facet: extended, test(node, state) {
                    dataCalls.push([...span(node), byteAt(state.selection.main.head)]);
                    return node.name === 'Keyword' && state.selection.main.head === 0;
                }},
                {type: 'replace', facet: replaced, test: node => node.name === 'Keyword'},
            ]}),
            identity.add({App: name}),
            styleTags({Number: tag}),
            foldNodeProp.add({App(node, state) {
                foldCalls.push([...span(node), byteAt(state.selection.main.head)]);
                if (!state.doc.sliceString(node.from, node.from + 7).startsWith('(module')) return null;
                const first = state.doc.lineAt(node.from).to;
                return {from: first - (state.selection.main.head === 0 ? 0 : 1), to: node.to - 1};
            }}),
            indentNodeProp.add({App(cx) {
                const next = cx.continue();
                indentCalls.push([...span(cx.node), byteAt(cx.pos), byteAt(cx.state.selection.main.head),
                    cx.unit, cx.baseIndent, next, cx.textAfter]);
                return cx.baseIndent + cx.unit + (next ?? 0);
            }}),
        ]}, name);
        return yamlFrontmatter({content: language});
    };
    const modes = {strict: configured('strict', true, tags.atom), recover: configured('recover', false, tags.bool)};
    const highlighter = HighlightStyle.define([{tag: tags.atom, class: 'atom'}, {tag: tags.bool, class: 'bool'}]);
    const slot = new Compartment(), loadService = new Compartment();
    const normal = text => text.replace(/\r\n?|\n/g, '\n');
    let source = '---\r\ntitle: 前😀\r---\r\n(module\r\n  (; é ;)(func\r    42)\r\n)';
    let state = EditorState.create({doc: source,
        extensions: [slot.of(modes.strict), loadService.of([]), root.of({probe: 'user'}),
            indentUnit.of('   '), syntaxHighlighting(highlighter)]});
    function capture(label, edit, mode, selection) {
        const offsets = projectedOffsets(source);
        byteAt = offsets.byteAt;
        const tree = syntaxTree(state);
        const body = tree.topNode.getChild('Module');
        assert(body);
        const nodes = [], highlights = [], metadata = [];
        const cursor = body.cursor(IterMode.IgnoreMounts);
        cursor.iterate(node => nodes.push([...span(node), node.type.prop(identity) ?? null]));
        highlightTree(tree, highlighter, (from, to, tag) => highlights.push([byteAt(from), byteAt(to), tag]));
        for (let at = 0; at <= offsets.units; at++) {
            try { byteAt(at); } catch { continue; }
            const before = state.languageDataAt('probe', at, -1);
            const after = state.languageDataAt('probe', at, 1);
            metadata.push([byteAt(at), before, after]);
        }
        const pos = state.doc.toString().indexOf('42');
        const closing = state.doc.toString().lastIndexOf(')');
        const indents = [pos, closing].map((at, i) => {
            const options = i ? {simulateBreak: at, simulateDoubleBreak: true} : {};
            const column = getIndentation(new IndentContext(state, options), at);
            return [byteAt(at), !!i, column, indentCalls.splice(0)];
        });
        const folds = Array.from({length: state.doc.lines}, (_, index) => {
            const line = state.doc.line(index + 1);
            const fold = foldable(state, line.from, line.to);
            return [byteAt(line.from), byteAt(line.to), fold && [byteAt(fold.from), byteAt(fold.to)], foldCalls.splice(0)];
        });
        const override = state.update({effects: loadService.reconfigure(Prec.high(foldService.of((_, from, to) =>
            from === body.from ? {from: to, to: body.to} : null)))}).state;
        const first = override.doc.lineAt(body.from);
        const priorityFold = foldable(override, first.from, first.to);
        assert.equal(foldCalls.length, 0);
        console.log(JSON.stringify({label, edit, mode, selection, source, body: [byteAt(body.from), byteAt(body.to)],
            nodes, highlights, metadata, dataCalls: dataCalls.splice(0), indents,
            folds, priorityFold: [byteAt(priorityFold.from), byteAt(priorityFold.to)]}));
    }
    function replay(label, {replace, mode, select, rejected} = {}) {
        let edit, changes, nextSource = source;
        if (replace) {
            const [before, insert] = replace;
            const start = source.indexOf(before), end = start + before.length;
            assert(start >= 0);
            const prefix = source.slice(0, start);
            changes = {from: normal(prefix).length, to: normal(source.slice(0, end)).length, insert: normal(insert)};
            edit = [Buffer.byteLength(prefix), Buffer.byteLength(source.slice(0, end)), insert];
            nextSource = prefix + insert + source.slice(end);
        }
        const at = select ? normal(nextSource).indexOf(select) : undefined;
        const selection = at === undefined ? undefined : projectedOffsets(nextSource).byteAt(at);
        const update = () => state.update({changes, selection: at === undefined ? undefined : {anchor: at},
            effects: mode ? slot.reconfigure(modes[mode]) : []}).state;
        if (rejected) {
            assert.throws(update, error => {
                const pos = /^No parse at (\d+)$/.exec(error.message);
                assert(pos, error.message);
                console.log(JSON.stringify({label, edit, mode, rejected: true, attempted: nextSource,
                    stop: projectedOffsets(nextSource).byteAt(Number(pos[1]))}));
                return true;
            });
        } else {
            state = update();
            source = nextSource;
            capture(label, edit, mode, selection);
        }
    }
    capture('strict');
    replay('move selection', {select: '42'});
    replay('front matter grows', {replace: ['前😀', 'Longer 前😀']});
    replay('strict rejects unmatched expression', {replace: ['42)', '42'], rejected: true});
    replay('enable recovery and delete closer', {replace: ['42)', '42'], mode: 'recover'});
    replay('repair and install strict', {replace: ['42\r\n', '42)\r\n'], mode: 'strict'});
    replay('body without front matter', {replace: [source.slice(0, source.indexOf('(module')), '']});
} finally {
    rmSync(scratch, {recursive: true, force: true});
}
