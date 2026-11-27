// Run with Node and the directory of SHA-512-pinned npm archives.
import assert from 'node:assert/strict';
import {mkdirSync, mkdtempSync, realpathSync, rmSync, symlinkSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {projectedOffsets} from './editor_line_offsets.mjs';
import {pinnedModules} from './pinned_modules.mjs';

const jsonNames = {
    closedBy: {Number: ['String', 'Null']},
    openedBy: {String: ['Number'], Null: ['Number']},
};
const lezerNames = {
    closedBy: {RuleName: ['CharClass', 'Literal']}, openedBy: {Literal: ['RuleName']},
};
const cases = [
    ['json', 'stock roles beat text options', '|{\r\n "😀": |[|0, "אב"|]|, "x": true\r\n|}|', {},
        {brackets: '()', maxScanDistance: 1}],
    ['json', 'two mate names', '[ |1|2|, |"א|ב"|, |2|3|, |null| ]', jsonNames],
    ['json', 'mate has no reverse role', '[ |12|, |"😀"| ]', {closedBy: jsonNames.closedBy}],
    ['json', 'opposite role mismatches', '[ |12|, |true|, "😀" ]', {
        closedBy: jsonNames.closedBy, openedBy: {True: ['False']},
    }],
    ['json', 'empty roles shadow punctuation', '|{|"😀":0|}|', {
        closedBy: {'{': []}, openedBy: {'}': []},
    }],
    ['json', 'plain scan uses UTF-16 distance', '"|[|😀|]|"', {}, {maxScanDistance: 3}],
    ['json', 'plain scan stops at limit', '"|[|😀|]|"', {}, {maxScanDistance: 2}],
    ['json', 'after-cursor disabled', '[ |12|, |"😀"| ]', jsonNames, {afterCursor: false}],
    ['lezer', 'stock roles beat text options', '@top Main |{\r\n foo |(|"😀"|)|\r\n|}|', {},
        {brackets: '[]', maxScanDistance: 1}],
    ['lezer', 'mate need not be first name', '@top Main {\r\n |f|oo| |"😀"|\r\n}', lezerNames],
    ['lezer', 'mate has no reverse role', '@top Main { |word| |"😀"| }', {
        closedBy: lezerNames.closedBy,
    }],
    ['lezer', 'empty list nests before an opposite role', '@top Main { |foo| |bar| |"😀"| }', {
        closedBy: {RuleName: []}, openedBy: {RuleName: ['Literal'], Literal: ['RuleName']},
    }],
    ['lezer', 'empty roles shadow punctuation', '@top Main |{|"😀"|}|', {
        closedBy: {'{': []}, openedBy: {'}': []},
    }],
    ['lezer', 'plain scan uses UTF-16 distance', '@top Main { "|[|😀|]|" }', {},
        {maxScanDistance: 3}],
    ['lezer', 'after-cursor disabled', '@top Main { |word| |"😀"| }', lezerNames,
        {afterCursor: false}],
];

assert.equal(process.argv.length, 3, 'usage: node lr_bracket_roles.mjs <pinned archive directory>');
const root = mkdtempSync(join(realpathSync(tmpdir()), 'lr-bracket-roles.'));
const oldPath = process.env.PATH;
try {
    const bin = join(root, 'bin');
    mkdirSync(bin);
    symlinkSync('/usr/bin/tar', join(bin, 'tar'));
    process.env.PATH = bin;
    const {load} = pinnedModules(resolve(process.argv[2]), root);
    const {jsonLanguage} = await load('@codemirror/lang-json');
    const {lezerLanguage} = await load('@codemirror/lang-lezer');
    const {NodeProp} = await load('@lezer/common');
    const {Compartment, EditorState} = await load('@codemirror/state');
    const {ViewPlugin} = await load('@codemirror/view');
    const {bracketMatching, matchBrackets, syntaxTree} = await load('@codemirror/language');
    const languages = {json: jsonLanguage, lezer: lezerLanguage};
    for (const [name, label, annotated, props, options = {}] of cases) {
        const parts = annotated.split('|');
        const source = parts.join('');
        const {byteAt, units} = projectedOffsets(source);
        let offset = 0;
        const carets = parts.slice(0, -1).map(part => offset += projectedOffsets(part).units);
        const language = languages[name].configure({props: [
            NodeProp.closedBy.add(type => props.closedBy?.[type.name]),
            NodeProp.openedBy.add(type => props.openedBy?.[type.name]),
        ]});
        const extension = bracketMatching(options);
        const plugin = extension.flat(Infinity).find(item => item instanceof ViewPlugin);
        assert.ok(plugin);
        const slot = new Compartment();
        const base = EditorState.create({doc: source, extensions: [language, slot.of(extension)]});
        assert.equal(base.doc.length, units);
        assert.equal(syntaxTree(base).length, units);
        const span = range => range ? [byteAt(range.from), byteAt(range.to)] : null;
        const hit = value => value ? [span(value.start), span(value.end), value.matched] : null;
        let caret;
        const capture = bracketMatching({...options, renderMatch(result) {
            caret = hit(result);
            return [];
        }});
        const queries = carets.map(head => {
            const state = base.update({selection: {anchor: head}}).state;
            const directional = [-1, 1].map(dir => hit(matchBrackets(state, head, dir, options)));
            const captured = state.update({effects: slot.reconfigure(capture)}).state;
            caret = null;
            plugin.create({state: captured});
            const marks = [];
            for (const it = plugin.create({state}).decorations.iter(); it.value; it.next()) {
                marks.push([...span(it), it.value.spec.class]);
            }
            return {head: byteAt(head), directional, caret, marks};
        });
        console.log(JSON.stringify({name, label, source, props, options, queries}));
    }
} finally {
    process.env.PATH = oldPath;
    rmSync(root, {recursive: true, force: true});
}
