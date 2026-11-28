// node crates/codemirror/tests/fixtures/xml_parser.mjs [pinned-archive-directory]
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {pinnedModules} from './pinned_modules.mjs';

const cases = [
    ['declarations, reference boundaries, and supplementary names', '\ufeff<?xml version="1.1"?>\r\n<!DOCTYPE r [<!ELEMENT r ANY]><ns:r11 xml:lang = "ja" α·β="😀&名;&#x1F600;&#123;" d\u0300=\'a<b>\'><𐀀/><\u{EFFFF}/><![cdata[α😀]]><?pi??><!--😀---></ns:r11>'],
    ['case, digits, and unfinished close tags', '<a1><a2>digit split</a3></a9><A></a></A><x></>text &ok;</1></x>'],
    ['ancestors close at a zero-width token', '<a>\r\n  <b>\r\n    <c>\r\n</a>\r\n<!DOCTYPE r>\r\n<r />'],
    ['a multiline opening tag does not hide a line-local fold', '<outer\r\n  attr="😀"><inner>start\r\n    more\r\n  </inner>\r\n</outer>'],
    ['bare tag recovers before the next element', '<a   <b/>'],
    ['incomplete attribute consumes the stray angle bracket', '<a x   <b/>'],
    ['quoted attribute followed by a stray angle bracket', '<a x="ok"   <b/>'],
    ['unquoted value consumes the unfinished suffix', '<a><b x=y </a>'],
    ['bad reference and missing quote get separate errors', '<a x="&bad &#x31; &another'],
    ['stray punctuation before a name', '<a???b/>'],
    ['a scalar outside the XML name range', '<a><\u{F0000}/></a>'],
    ['partial delimiter with non-ASCII content', '<a>\r\n  <![CDATA[é]>tail]]'],
    ['two punctuation bytes before a final self-close delimiter', '<a x=()/>'],
    ['stray angle bracket before a final self-close delimiter', '<a x=[</>'],
    ['final self-close recovery preserves raw UTF-8 and CRLF offsets', 'é😀\r\n<漢😀 x=\'é\'\r\n𐀀=\r\n[]/>'],
    ['whitespace before the delimiter keeps the unfinished opening tag', '<a x=() />'],
    ['an earlier attribute error keeps the unfinished opening tag', '<a &x=()/>'],
    ['spaces between invalid punctuation split the attribute errors', '<a x=( )/>'],
    ['mixed brackets and horizontal XML whitespace', '<a y="ok" x={\t ]/>'],
    ['CRLF between errors preserves raw UTF-8 offsets', 'é😀\r\n<漢😀 𐀀=(\r\n\t )/>'],
    ['space before the first punctuation keeps the unfinished opening tag', '<a x= ( )/>'],
    ['space before the delimiter does not recover spaced punctuation', '<a x=( ) />'],
    ['space before the angle bracket keeps the incomplete close tag', '<a x=[ </>'],
    ['a newline after the delimiter keeps the unfinished opening tag', '<a x=( )/>\r\n'],
    ['nested spaced punctuation keeps the unfinished opening tag', '<r><a x=( )/></r>'],
    ['quoted punctuation is a valid attribute value', '<a x="( )"/>'],
    ['a number and punctuation do not displace the final self-close', '<a x=1 )/>'],
    ['two kinds of punctuation leave two errors and the self-close', '<a x=? )/>'],
    ['adjacent invalid digits share an error', '<a x=12/>'],
    ['attribute recovery after supplementary text and raw CRLF', 'é😀\r\n<漢 x=1 )/>'],
];

const scratch = mkdtempSync(join(realpathSync(tmpdir()), 'gpui-xml-parser.'));
try {
    const {load} = pinnedModules(resolve(process.argv[2] ?? join(scratch, 'archives')), scratch);
    const {xml, xmlLanguage} = await load('@codemirror/lang-xml');
    const {EditorState} = await load('@codemirror/state');
    const {bracketMatchingHandle, foldable, getIndentation} = await load('@codemirror/language');
    const {highlightTree, tagHighlighter, tags} = await load('@lezer/highlight');
    const names = ['content', 'angleBracket', 'tagName', 'invalid', 'attributeName', 'attributeValue',
        'definitionOperator', 'character', 'blockComment', 'processingInstruction', 'documentMeta'];
    const highlighter = tagHighlighter([
        ...names.map(name => ({tag: tags[name], class: name})),
        {tag: tags.special(tags.string), class: 'specialString'},
    ]);
    for (const [label, source] of cases) {
        const byte = pos => Buffer.byteLength(source.slice(0, pos));
        const tree = xmlLanguage.parser.parse(source);
        const nodes = [], highlights = [], handles = [], lines = [];
        let depth = 0;
        tree.iterate({enter({node, name, from, to}) {
            nodes.push([depth++, name, byte(from), byte(to)]);
            const handle = node.type.prop(bracketMatchingHandle)?.(node);
            if (handle) handles.push([byte(from), byte(to), byte(handle.from), byte(handle.to)]);
        }, leave() { depth--; }});
        highlightTree(tree, highlighter, (from, to, tag) => highlights.push([byte(from), byte(to), tag]));

        // EditorState normalizes newlines. Use the original bytes for fold ranges.
        const state = EditorState.create({doc: source, extensions: [xml()]});
        const crlf = [...source.matchAll(/\r\n/g)].map((match, i) => match.index - i);
        const original = pos => byte(pos + crlf.filter(at => at < pos).length);
        for (let i = 1; i <= state.doc.lines; i++) {
            const line = state.doc.line(i);
            const fold = foldable(state, line.from, line.to);
            lines.push([getIndentation(state, line.from), fold ? [original(fold.from), original(fold.to)] : null]);
        }
        console.log(JSON.stringify({label, source, nodes, highlights, handles, lines}));
    }
} finally {
    rmSync(scratch, {recursive: true, force: true});
}
