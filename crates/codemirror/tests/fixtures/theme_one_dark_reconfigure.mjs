// Bundle with the SHA-512-verified upstream packages and run in Chromium.
// Read the JSON in #results. Copy these observations into the fixture with
// the tarball provenance. No generated CSS class names are stored.
import {defaultHighlightStyle, HighlightStyle, highlightingFor, syntaxHighlighting} from '@codemirror/language';
import {Compartment, Prec, StateEffect} from '@codemirror/state';
import * as oneDarkPackage from '@codemirror/theme-one-dark';
import {EditorView} from '@codemirror/view';
import {NodeType} from '@lezer/common';
import {tags} from '@lezer/highlight';

const {oneDark, oneDarkTheme, oneDarkHighlightStyle} = oneDarkPackage;
const scope = NodeType.define({id: 1, name: 'Section', top: true});
const otherScope = NodeType.define({id: 2, name: 'Other', top: true});
const syntax = (specs, options, installOptions) =>
  syntaxHighlighting(HighlightStyle.define(specs, options), installOptions);
const dark = syntax([{tag: [tags.keyword, tags.punctuation], color: '#008080', fontStyle: 'italic'}],
                    {themeType: 'dark'});
const extensions = {
  empty: [],
  theme: oneDarkTheme,
  highlight: syntaxHighlighting(oneDarkHighlightStyle),
  preset: oneDark,
  dark,
  darkHigh: Prec.high(dark),
  scoped: syntax([{tag: tags.keyword, color: '#009966', fontWeight: 'bold'}], {scope}),
  punctuationFallback: syntax([{tag: tags.punctuation, color: '#112233'}], {}, {fallback: true}),
};
const slots = {view: new Compartment(), highlight: new Compartment(), extra: new Compartment()};
const source = 'one dark';
const selection = {anchor: 1, head: 4};
const view = new EditorView({
  doc: source, selection, parent: document.body,
  extensions: Object.values(slots).map(slot => slot.of([])),
});
const color = css => {
  const [r, g, b, a = 1] = css.match(/[\d.]+/g).map(Number);
  return [r, g, b, Math.round(a * 255)].map(n => n.toString(16).padStart(2, '0')).join('');
};
function properties(tag, scope) {
  const cls = highlightingFor(view.state, [tag], scope);
  if (!cls) return {};
  const base = getComputedStyle(view.contentDOM).color;
  const probe = document.createElement('span');
  probe.className = cls;
  probe.textContent = 'x';
  view.contentDOM.appendChild(probe);
  const css = getComputedStyle(probe), result = {};
  if (css.color !== base) result.color = color(css.color);
  if (css.fontWeight === '700') result.fontWeight = css.fontWeight;
  if (css.fontStyle !== 'normal') result.fontStyle = css.fontStyle;
  if (css.textDecorationLine !== 'none') result.textDecorationLine = css.textDecorationLine;
  probe.remove();
  return result;
}
const steps = [];
function step(name, ...changes) {
  view.dispatch({effects: changes.map(([slot, value]) => slot === 'root'
    ? StateEffect.appendConfig.of(syntaxHighlighting(defaultHighlightStyle, {fallback: true}))
    : slots[slot].reconfigure(extensions[value]))});
  const range = view.state.selection.main;
  if (view.state.doc.toString() !== source || range.anchor !== selection.anchor || range.head !== selection.head)
    throw Error('reconfiguring modified the document or selection');
  const css = getComputedStyle(view.dom);
  steps.push({name, changes, dark: view.state.facet(EditorView.darkTheme),
              background: color(css.backgroundColor), foreground: color(css.color),
              keyword: properties(tags.keyword), punctuation: properties(tags.punctuation),
              link: properties(tags.link), inScope: properties(tags.keyword, scope),
              otherScope: properties(tags.keyword, otherScope)});
}
step('bare');
step('appendFallback', ['root', 'appendFallback']);
step('themeOnly', ['view', 'theme']);
step('splitBoth', ['highlight', 'highlight']);
step('themeSurvivesHighlightRemoval', ['highlight', 'empty']);
step('splitRestored', ['highlight', 'highlight']);
step('highlightOnly', ['view', 'empty']);
step('fallbackAfterRemoval', ['highlight', 'empty']);
step('darkLayerInLight', ['extra', 'dark']);
step('darkLayerInDark', ['view', 'theme']);
step('oneDarkFirst', ['highlight', 'highlight']);
step('darkLayerFirst', ['extra', 'darkHigh']);
step('highlightWithoutTheme', ['view', 'empty']);
step('scopeOnly', ['highlight', 'empty'], ['extra', 'scoped']);
step('presetWithScope', ['view', 'preset']);
step('fallbackRestored', ['view', 'empty'], ['extra', 'empty']);
step('firstFallback', ['extra', 'punctuationFallback']);
step('presetSuppressesFirstFallback', ['view', 'preset']);
view.destroy();

const results = document.body.appendChild(document.createElement('pre'));
results.id = 'results';
results.textContent = JSON.stringify({exports: Object.keys(oneDarkPackage).sort(), source, selection, steps});
