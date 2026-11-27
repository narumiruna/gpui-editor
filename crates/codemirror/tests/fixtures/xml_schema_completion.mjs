// Run with the dependencies from docs/codemirror-upstream.json installed.
// Node prints the source fixtures. Bundle this module for a browser and load it
// in an empty HTML document to check insertion through the upstream editor.
import { autocompletion, acceptCompletion, CompletionContext, currentCompletions, startCompletion } from "@codemirror/autocomplete";
import { xml } from "@codemirror/lang-xml";
import { EditorState } from "@codemirror/state";
import { EditorView } from "@codemirror/view";

const schema = {
    elements: [
        { name: "root", top: true, children: ["zeta", "unknown", "alpha"],
          textContent: ["yes", "no"], completion: { detail: "Root", boost: 1 },
          attributes: ["shared", "undeclared", { name: "mode", values: [
              "one", { label: "okay", apply: '"custom"', detail: "Override" },
              { label: '"open"', type: "enum" }
          ] }] },
        { name: "alpha", children: [] },
        { name: "zeta" },
        { name: "envelope", top: true },
    ],
    attributes: [
        { name: "shared", values: ["shared value"], completion: { detail: "Shared" } },
        { name: "z-global", global: true },
        { name: "a-global", global: true },
        { name: "mode", values: ["fallback"] },
    ],
};

// [schema enabled, marked text, explicit, optional expected accepted document]
// null means the source options do not match the typed prefix in the editor.
const cases = [
    [false, "<root>|", true, "<root></root>"],
    [false, "<r|", false],
    [false, "<root>|</root>", false],
    [false, "<root>&a|</root>", false],
    [true, "|", true],
    [true, "<r|", false, "<root"],
    [true, "<root><|</root>", false],
    [true, "<root> |", true, "<root> </root>"],
    [true, "<root>|</root>", true],
    [true, "<alpha><|</alpha>", true],
    [true, "<unlisted><|</unlisted>", true],
    [true, "<root |>", false],
    [true, "<root sha|>", true, "<root shared>"],
    [true, "<unlisted |>", false],
    [true, "<root mode=|>", true],
    [true, '<root mode="ok|"/>', true, '<root mode="custom"/>'],
    [true, '<root mode="one|"/>', true, '<root mode="one"/>'],
    [true, "<root mode='o|' />", true, null],
    [true, '<unlisted mode="f|"/>', true, '<unlisted mode="fallback"/>'],
    [true, "<root>😀</ro|>", true, "<root>😀</root>"],
];

function query([enabled, marked, explicit, inserted]) {
    const pos = marked.indexOf("|");
    const doc = marked.replace("|", "");
    const state = EditorState.create({
        doc, selection: { anchor: pos },
        extensions: [xml(enabled ? schema : {}), autocompletion({ interactionDelay: 0 })],
    });
    const sources = state.languageDataAt("autocomplete", pos, -1);
    if (sources.length !== 1) throw new Error(`Expected one source for ${marked}`);
    const result = sources[0](new CompletionContext(state, pos, explicit));
    const fixture = {
        enabled, marked, explicit,
        result: result && {
            from: new TextEncoder().encode(doc.slice(0, result.from)).length,
            to: result.to == null ? null : new TextEncoder().encode(doc.slice(0, result.to)).length,
            options: result.options.map(({ label, type = null, detail = null, boost = 0, apply = null }) =>
                ({ label, type, detail, boost, apply })),
        },
    };
    return { state, fixture, inserted };
}

if (typeof document === "undefined") {
    for (const entry of cases) console.log(JSON.stringify(query(entry).fixture));
} else {
    const outcome = document.body.appendChild(document.createElement("pre"));
    try {
        let count = 0, rejected = 0;
        for (const entry of cases) {
            const { state, fixture, inserted } = query(entry);
            if (inserted === undefined) continue;
            const view = new EditorView({ state, parent: document.body });
            view.focus();
            startCompletion(view);
            for (let i = 0; i < 30 && !currentCompletions(view.state).length; i++) {
                await new Promise(resolve => setTimeout(resolve, 20));
            }
            if (inserted === null) {
                if (currentCompletions(view.state).length || acceptCompletion(view)) {
                    throw new Error(`${fixture.marked}: unexpected single-quote match`);
                }
                rejected++;
            } else {
                if (!acceptCompletion(view) || view.state.doc.toString() !== inserted) {
                    throw new Error(`${fixture.marked}: inserted ${view.state.doc.toString()}`);
                }
                count++;
            }
            view.destroy();
        }
        outcome.textContent = `PASS: ${count} live insertions, ${rejected} rejected prefix`;
    } catch (error) {
        outcome.textContent = `FAIL: ${error.message}`;
    }
}
