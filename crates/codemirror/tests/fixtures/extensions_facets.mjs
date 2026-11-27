// Run with Node and the path to the hash-verified state package's dist/index.js.
import { pathToFileURL } from 'node:url';

const { Compartment, EditorState, Facet, StateEffect, StateField } =
  await import(pathToFileURL(process.argv[2]));

function dependencies() {
  const calls = [0, 0, 0, 0, 0, 0];
  const doc = Facet.define(), selection = Facet.define(), constant = Facet.define();
  const projected = Facet.define(), setting = Facet.define(), derived = Facet.define();
  const set = StateEffect.define();
  const field = StateField.define({
    create: () => ({ n: 0 }),
    update: (old, tr) => ({ n: tr.effects.find(e => e.is(set))?.value ?? old.n }),
    compare: (old, next) => old.n === next.n,
  });
  const fieldSlot = new Compartment(), settingSlot = new Compartment(), extra = new Compartment();
  const settingA = setting.of({ n: 10 }), settingB = setting.of({ n: 20 });
  const additionalDoc = doc.compute(['doc'], s => ({ n: 100 * s.doc.length + ++calls[5] }));
  let state = EditorState.create({doc: 'ab', extensions: [
    derived.compute([projected.reader, setting.reader], s => {
      calls[4]++;
      return { n: s.facet(projected)[0].n + s.facet(setting)[0].n };
    }),
    fieldSlot.of(field), settingSlot.of(settingA), extra.of([]),
    doc.compute(['doc'], s => { calls[0]++; return { n: s.doc.length }; }),
    selection.compute(['selection'], s => { calls[1]++; return { n: s.selection.main.head }; }),
    constant.compute([], () => { calls[2]++; return { n: 99 }; }),
    projected.from(field, v => { calls[3]++; return { n: v.n }; }),
  ]});
  let old = [];
  const read = () => {
    const outputs = [doc, selection, constant, projected, setting, derived].map(f => state.facet(f));
    const step = { calls: [...calls], values: outputs.map(list => list.map(v => v.n)),
                   reused: outputs.map((value, i) => value === old[i]) };
    old = outputs;
    return step;
  };
  const steps = [read()];
  for (const spec of [
    {}, { selection: { anchor: 0 } }, { selection: { anchor: 1 } },
    { changes: { from: 2, insert: 'c' } }, { effects: set.of(1) },
    { effects: set.of(1) },
    { effects: settingSlot.reconfigure(settingA) },
    { effects: settingSlot.reconfigure(settingB) },
    { effects: settingSlot.reconfigure(settingB) },
    { effects: fieldSlot.reconfigure(field.init(() => ({ n: 2 }))), changes: { from: 3, insert: 'd' } },
    { effects: extra.reconfigure(additionalDoc), changes: { from: 4, insert: 'e' } },
    { effects: extra.reconfigure([]) },
    { effects: extra.reconfigure(additionalDoc) },
  ]) {
    state = state.update(spec).state;
    steps.push(read());
  }
  return steps;
}

function comparisons() {
  let events = [];
  const tokens = Facet.define({
    combine: values => {
      const labels = values.map(v => v.label).join(',');
      events.push(['combine', labels]);
      return { sum: values.reduce((sum, v) => sum + v.n, 0), labels };
    },
    compareInput: (next, old) => {
      events.push(['input', next.label, old.label]);
      return next.n === old.n;
    },
    compare: (next, old) => {
      events.push(['output', next.labels, old.labels]);
      return next.sum === old.sum;
    },
  });
  const derived = Facet.define();
  const single = () => tokens.compute(['doc'], state => {
    const name = state.doc.toString();
    events.push(['single', name]);
    return { n: 'ab'.includes(name) ? 1 : 2, label: `s:${name}` };
  });
  const first = single();
  const multi = tokens.computeN(['doc'], state => {
    const name = state.doc.toString();
    events.push(['multi', name]);
    const values = 'ab'.includes(name) ? [1, 2] : name === 'c' ? [1, 1] : name === 'f' ? [] : [3];
    return values.map((n, i) => ({n, label: `m:${name}:${i}`}));
  });
  const fixed = tokens.of({ n: 10, label: 'fixed' });
  const project = derived.compute([tokens], state => {
    const value = state.facet(tokens);
    events.push(['dependent', value.labels]);
    return value;
  });
  const slot = new Compartment();
  let state = EditorState.create({doc: 'a', extensions: [slot.of([first, fixed, multi]), project]});
  let prev, prevDerived;
  const read = () => {
    const value = state.facet(tokens), dependent = state.facet(derived);
    const step = { value, reused: value === prev, derivedReused: dependent === prevDerived, events };
    prev = value;
    prevDerived = dependent;
    events = [];
    return step;
  };
  const steps = [read()];
  for (const spec of [
    ...[...'bcdef'].map(insert => ({ changes: { from: 0, to: 1, insert } })),
    { effects: slot.reconfigure([multi, first, fixed]) },
    { effects: slot.reconfigure([multi, single(), fixed]) },
    { effects: slot.reconfigure([tokens.of({n: 2, label: 'static'}), fixed]) },
    { effects: slot.reconfigure([first, fixed, multi]) },
  ]) {
    state = state.update(spec).state;
    steps.push(read());
  }
  return steps;
}

function staticAndDefault() {
  let calls = [];
  const facet = Facet.define({
    combine: inputs => {
      const tag = inputs.map(x => x.tag).join(',');
      calls.push(['combine', tag]);
      return { n: inputs.reduce((sum, x) => sum + x.n, 0), tag };
    },
    compareInput: () => { throw new Error('Static inputs are never compared'); },
    compare: (next, old) => {
      calls.push(['compare', next.tag, old.tag]);
      return next.n === old.n;
    },
  });
  const collected = Facet.define();
  const collectEmpty = collected.computeN([], () => []);
  const elem = collected.of({ n: 1 });
  const equivalent = collected.of({ n: 1 });
  const zero = facet.of({n: 0, tag: 'zero'});
  const one = facet.of({n: 1, tag: 'one'});
  let state = EditorState.create();
  let prev, prevCollect;
  const steps = [];
  const record = () => {
    const value = state.facet(facet), list = state.facet(collected);
    steps.push({value, reused: value === prev, list: list.map(x => x.n),
                listReused: list === prevCollect, calls});
    prev = value;
    prevCollect = list;
    calls = [];
  };
  record();
  for (const extensions of [[zero, collectEmpty], [zero, elem, collectEmpty], [one, elem],
                            [one, equivalent], [zero, equivalent], []]) {
    state = state.update({ effects: StateEffect.reconfigure.of(extensions) }).state;
    record();
  }
  return steps;
}

console.log(JSON.stringify({dependencies: dependencies(), comparisons: comparisons(),
                            staticAndDefault: staticAndDefault()}, null, 2));
