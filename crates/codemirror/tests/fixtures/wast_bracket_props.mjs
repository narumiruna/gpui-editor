// Run with Node and the directory of SHA-512-pinned npm archives.
import assert from 'node:assert/strict';
import {mkdirSync, mkdtempSync, realpathSync, rmSync, symlinkSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {projectedOffsets} from './editor_line_offsets.mjs';
import {pinnedModules} from './pinned_modules.mjs';

const paired = {
    closedBy: {Keyword: ['String'], Identifier: ['Number']},
    openedBy: {String: ['Keyword'], Number: ['Identifier']},
};
const apps = {closedBy: {App: ['App']}, openedBy: {App: ['App']}, handle: {App: 'keyword'}};
const cases = [
    ['stock syntax beats text options', '|(|module\r\n (data "[|😀|]")|)|', {},
        {brackets: '[]', maxScanDistance: 1}],
    ['two partner names', '(mo|dule |$x|)\r\n(fu|nc |123|)', {
        closedBy: {Keyword: ['Identifier', 'Number']},
        openedBy: {Identifier: ['Keyword'], Number: ['Keyword']},
    }, {brackets: '[]', maxScanDistance: 1}],
    ['mate has no role', '(mo|dule |$x|)', {closedBy: {Keyword: ['Identifier']}}],
    ['empty lists shadow text options', '|(|func|)|', {closedBy: {'(': []}, openedBy: {')': []}}],
    ['missing list allows opposite text role', '|(|func|)|', {}, {brackets: ')('}],
    ['missing named role is inactive', '(fu|nc $x)', {}],
    ['empty named role starts an unmatched scan', '(fu|nc $x)', {closedBy: {Keyword: []}}],
    ['empty list can still nest', '(mo|dule $x 0 "en|d")', {
        ...paired, closedBy: {...paired.closedBy, Identifier: []},
    }],
    ['balance same direction siblings', '(mo|dule $x i32 1 "en|d")', paired],
    ['nest before checking other direction', '(mo|dule $x 1 "en|d")', {
        ...paired, openedBy: {...paired.openedBy, Identifier: ['Type']},
    }],
    ['mismatched closing sibling', '(mo|dule $x "mid" |1| "en|d")', paired],
    ['named sibling wins before depth role; child handles', '(mo|dule\r\n |(|fu|nc| 0|)| (|da|ta| "😀"))', apps,
        {brackets: '[]', maxScanDistance: 1}],
    ['mismatch uses endpoint handle', '(module "st|art" (da|ta "😀"))', {
        closedBy: {String: ['Number']}, openedBy: {App: ['Keyword']}, handle: apps.handle,
    }],
    ['child handle caret policy before only', '(module\r\n |(|fu|nc| 0) (|da|ta| "😀"))', apps,
        {afterCursor: false}],
    ['null endpoint remains matched', '(module (fu|nc 0) |"😀"|)', {
        ...apps, closedBy: {App: ['String']}, openedBy: {String: ['App']},
        handle: {App: 'keyword', String: 'none'},
    }],
    ['null mismatch endpoint stays unmatched', '(module (fu|nc 0) |"😀"|)', {
        ...apps, closedBy: {App: ['Identifier']}, openedBy: {String: ['App']},
        handle: {App: 'keyword', String: 'none'},
    }],
    ['null start continues to parent', '(mo|dule 0) (func)', {
        ...apps, closedBy: {...apps.closedBy, Keyword: []},
        handle: {...apps.handle, Keyword: 'none'},
    }],
    ['null starts fall back to isolated text', '(data "|[|😀|]|" "]")', {
        closedBy: {String: ['String']}, openedBy: {String: ['String']}, handle: {String: 'none'},
    }],
    ['null starts cannot match text in another syntax type', '(data "|[|😀" ;; ]\r\n)', {
        closedBy: {String: []}, handle: {String: 'none'},
    }],
    ['explicit null property keeps whole node', '(mo|dule |$x|)', {
        closedBy: {Keyword: ['Identifier']}, openedBy: {Identifier: ['Keyword']},
        handle: {Keyword: null},
    }],
    ['plain scan at normalized UTF-16 limit', '(; |[😀\r\n]| ;)\r[ ]', {}, {maxScanDistance: 4}],
    ['plain scan exceeds normalized UTF-16 limit', '(; |[😀\r\n]| ;)\r[ ]', {}, {maxScanDistance: 3}],
    ['empty endpoint is not paintable', '(module 0|) (func', {
        closedBy: {App: ['App']}, handle: {App: 'lastChild'},
    }],
];

assert.equal(process.argv.length, 3, 'usage: node wast_bracket_props.mjs <pinned archive directory>');
const root = mkdtempSync(join(realpathSync(tmpdir()), 'wast-bracket-props.'));
const oldPath = process.env.PATH;
try {
    // Do not let the shared loader fetch an archive if one is missing.
    const bin = join(root, 'bin');
    mkdirSync(bin);
    symlinkSync('/usr/bin/tar', join(bin, 'tar'));
    process.env.PATH = bin;
    const {load} = pinnedModules(resolve(process.argv[2]), root);
    const {wastLanguage} = await load('@codemirror/lang-wast');
    const {NodeProp} = await load('@lezer/common');
    const {Compartment, EditorState} = await load('@codemirror/state');
    const {ViewPlugin} = await load('@codemirror/view');
    const {bracketMatching, bracketMatchingHandle, matchBrackets, syntaxTree} = await load('@codemirror/language');
    const handles = {
        keyword: node => node.getChild('Keyword'),
        none: () => null,
        lastChild: node => node.lastChild,
    };
    for (const [name, annotated, props, options = {}] of cases) {
        const parts = annotated.split('|');
        const source = parts.join('');
        const {byteAt, units} = projectedOffsets(source);
        let offset = 0;
        const carets = parts.slice(0, -1).map(part => offset += projectedOffsets(part).units);
        const language = wastLanguage.configure({props: [
            NodeProp.closedBy.add(type => props.closedBy?.[type.name]),
            NodeProp.openedBy.add(type => props.openedBy?.[type.name]),
            bracketMatchingHandle.add(type => {
                const key = props.handle?.[type.name];
                return key === null ? null : handles[key];
            }),
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
            let marks = [];
            try {
                for (const it = plugin.create({state}).decorations.iter(); it.value; it.next()) {
                    marks.push([...span(it), it.value.spec.class]);
                }
            } catch (err) {
                assert.equal(name, 'empty endpoint is not paintable');
                assert.equal(err.message, 'Mark decorations may not be empty');
                marks = err.message;
            }
            return {head: byteAt(head), directional, caret, marks};
        });
        console.log(JSON.stringify({name, source, props, options, queries}));
    }
} finally {
    process.env.PATH = oldPath;
    rmSync(root, {recursive: true, force: true});
}
