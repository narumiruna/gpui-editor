// Run with capture_browser_fixture.mjs and the verified @codemirror/lang-java archives.
import {java} from '@codemirror/lang-java';
import {bidiIsolates, forceParsing, syntaxTree} from '@codemirror/language';
import {EditorState} from '@codemirror/state';
import {EditorView} from '@codemirror/view';
import {NodeProp} from '@lezer/common';

const cases = [
    {name: 'default', explicit: false, source: 'class C { String s = "אב"; // عرب\n }'},
    {name: 'comments-and-strings', source: 'class C { String s = "אב"; // عرب\r\n /* גד\r\n abc */ char p = \'ז\'; int x = 0;\r\n }'},
    {name: 'mixed-lines', source: '// עברית\rclass C {\r\n  String s = "😀 عرب";\r  // more 中文\r\n }'},
    {name: 'ascii', source: 'class C { String s = "abc"; // ASCII\n }'},
    {name: 'always', always: true, source: 'class C { String s = "abc"; // ASCII\n }'},
    {name: 'text-block', always: true, source: 'class C { String s = """\r\nאב😀\r\n abc\r\n"""; }'},
    {name: 'missing-end', source: 'class C {\r\n String s = "אב😀;\r\n}'},
];
const result = [];
for (const {name, source, explicit = true, always = false} of cases) {
    const view = new EditorView({parent: document.body, state: EditorState.create({
        doc: source,
        extensions: [java(), explicit ? bidiIsolates({alwaysIsolate: always}) : []],
    })});
    if (!forceParsing(view, view.state.doc.length, 1000)) throw new Error('no syntax');
    view.measure();
    const offsets = [0];
    const encoder = new TextEncoder();
    for (const part of source.match(/\r\n|[^]/gu)) {
        const previous = offsets.at(-1);
        if (part.length === 2 && part !== '\r\n') offsets.push(null);
        offsets.push(previous + encoder.encode(part).length);
    }
    const marks = [];
    for (const get of view.state.facet(EditorView.bidiIsolatedRanges)) {
        for (let mark = get(view).iter(); mark.value; mark.next()) {
            marks.push([offsets[mark.from], offsets[mark.to], mark.value.spec.attributes.dir]);
        }
    }
    const annotated = [];
    syntaxTree(view.state).iterate({enter(node) {
        if (node.type.prop(NodeProp.isolate) !== undefined) {
            annotated.push([node.name, offsets[node.from], offsets[node.to]]);
        }
    }});
    result.push({name, source, explicit, always, annotated, marks});
    view.destroy();
}
globalThis.nativeSupport = result.map(row => JSON.stringify(row));
