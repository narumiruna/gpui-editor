// Run from the repository root: node crates/codemirror/tests/fixtures/state_create.mjs <archive directory>
import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import assert from 'node:assert/strict';

const manifest = JSON.parse(readFileSync('docs/codemirror-upstream.json', 'utf8'));
const { name, version, integrity } = manifest.packages.find(entry => entry.name === '@codemirror/state');
const directory = resolve(process.argv[2], '@codemirror__state');
const archive = resolve(directory, `state-${version}.tgz`);
const index = resolve(directory, 'package/dist/index.js');
assert.equal(`sha512-${createHash('sha512').update(readFileSync(archive)).digest('base64')}`, integrity);
assert.deepEqual(readFileSync(index), execFileSync('tar', ['-xOf', archive, 'package/dist/index.js']));
const { Compartment, EditorSelection, EditorState, Facet, Prec, StateField, Text } =
  await import(pathToFileURL(index));

const line = value => EditorState.lineSeparator.of(value);
const allow = value => EditorState.allowMultipleSelections.of(value);
const selection = EditorSelection.create([EditorSelection.cursor(0), EditorSelection.range(5, 3)], 1);
const describe = state => ({
  text: state.doc.toString(), lines: state.doc.toJSON(), line_break: state.lineBreak,
  display: state.sliceDoc(), multiple: state.facet(EditorState.allowMultipleSelections),
  selection: state.selection.toJSON(),
});

function callbacks() {
  const events = [];
  const setting = Facet.define({ combine: inputs => {
    if (inputs.length) events.push(['static', ...inputs]);
    return inputs.join(',');
  }});
  const dynamic = Facet.define({ combine: inputs => {
    if (inputs.length) events.push(['dynamic-combine', ...inputs]);
    return inputs[0];
  }});
  const update = () => assert.fail('Creation must not run field updates');
  const second = StateField.define({ create: state => {
    events.push(['second', state.facet(setting)]);
    return 'second';
  }, update });
  const first = StateField.define({ create: () => assert.fail('Default initializer ran'), update });
  const high = setting.of('high');
  const compartment = new Compartment();
  const state = EditorState.create({
    doc: 'aa::b|c::', selection,
    extensions: [
      dynamic.compute([first, second], state => {
        events.push(['dynamic-compute', state.selection.ranges.length]);
        return `${state.field(first)}:${state.field(second)}`;
      }),
      second, setting.of('low'), high, line('|'),
      first.init(() => assert.fail('Lower precedence initializer ran')),
      compartment.of(Prec.high([
        high, line('::'), first.init(state => {
          events.push(['first', state.doc.toString(), state.selection.toJSON(), state.lineBreak]);
          return 'first';
        }),
      ])),
      allow(false),
    ],
  });
  return { ...describe(state), events, dynamic: state.facet(dynamic) };
}

function invalidSelection() {
  const events = [];
  const setting = Facet.define({ combine: inputs => {
    if (inputs.length) events.push('static');
    return inputs;
  }});
  const field = StateField.define({ create: () => events.push('field'), update: old => old });
  let error;
  try {
    EditorState.create({
      doc: 'a\r\nb',
      selection: EditorSelection.create([EditorSelection.cursor(0), EditorSelection.cursor(4)]),
      extensions: [setting.of(true), field, allow(false)],
    });
  } catch (e) {
    error = e.name;
  }
  assert.equal(error, 'RangeError');
  return { error, events };
}

const text = Text.of(['a|b', 'c']);
const persistent = EditorState.create({ doc: text, extensions: line('|'), selection: { anchor: 2 } });
const source = 'a\r\nb\rc\nd';
console.log(JSON.stringify({
  pin: { name, version, integrity },
  empty: describe(EditorState.create()),
  default_lines: describe(EditorState.create({ doc: source })),
  empty_separator: describe(EditorState.create({ doc: source, extensions: line('') })),
  callbacks: callbacks(),
  invalid_selection: invalidSelection(),
  persistent: { ...describe(persistent), same_text: persistent.doc === text },
  allow_conflict: describe(EditorState.create({
    doc: 'abcdef', selection, extensions: [Prec.high(allow(false)), allow(true)],
  })),
  default_multiple: describe(EditorState.create({ doc: 'abcdef', selection })),
  unmatched_lf: describe(EditorState.create({ doc: 'a\nb\r\nc', extensions: line('\r\n') })),
}, null, 2));
