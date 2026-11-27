# GPUI Editor CodeMirror core

`gpui-editor-codemirror` provides text, transactions, history, commands, keymaps, language services, and other editor components. It does not depend on GPUI, a browser, or JavaScript. It is a partial port of CodeMirror 6, not a replacement for the full JavaScript API. See the [migration coverage](../../docs/codemirror-migration-coverage.md) for the ported features, gaps, and verification results.

## Edit and history rules

- All offsets are UTF-8 bytes. Edits must start and end at character boundaries. `Text` keeps LF and CRLF bytes as given. Logical line ranges omit their line endings.
- A change set contains non-overlapping replacements in the old document. A transaction can set a selection in the new document. If it does not, the transaction maps the old selection. An invalid edit or selection leaves the state unchanged.
- A cursor stays before text inserted at its position. For a nonempty range, the start maps after an insertion at the start, and the end maps before an insertion at the end. The range keeps its direction. If a replacement strictly covers both endpoints, Rust collapses the range to the start of the replacement. Upstream can map the endpoints across each other in this case.
- History stores change sets and their inverses, with the before and after selections and caller metadata. It keeps up to 100 undo groups. Without an explicit group or a timestamp annotation, each text edit creates one undo step. Selection-only updates do not enter history. A new text edit clears redo.

## Language services

Use `LanguageDescription::native_support_load` to load a catalog entry. Parse with the returned `NativeLanguageSupport` to query highlights, language data, and default completion sources. Results use source byte offsets, including in mounted syntax. Completion edits cannot cross excluded source gaps. Hosts must apply edits, run input handlers, and update their UI.

Use `override_editor_surface` to install either editor color independently. The first value for each color in extension precedence order wins, including the One Dark editor colors. Read the result with `editor_surface_colors`. An override does not select a light or dark appearance or replace other view roles. Use `HighlightRule::new` with `with_resets`, or `HighlightStyle::with_all_resets`, to cancel competing weight, style, and decoration on the same span. These resets do not change the host's base font or decoration. Use `HighlightTag::define` for a distinct custom tag, or `HighlightTag::derive` to add a less-specific parent. Use `HighlightSelector::from(&tag)` to select the definition, not its name. `HighlightTag::new` still makes a named tag. Define a custom modifier with `CustomTagModifier::define`. Apply it with `with_custom_modifier` and build a selector from the modified tag. Repeated application and application order do not change the result. Use `HighlightStyle::with_wast_node_type_scope` or `with_wast_facet_scope` for WAST identity scopes. Resolve them with `highlighting_for_context`. Read the context from the snapshot that supplies the tags, not from a fence label or an editor caret. A viewport scope requires published coverage.

Use `go_language_extension()` for Go syntax and metadata, or `go_extension()` to add the default fixed and local completion sources. Hosts can query the sources as `GoCompletionSource` data. Neither extension installs a completion UI, keys, bracket matching, or input handlers.

WAST configurations made with `with_node_props` expose typed properties on raw, synchronous, and viewport trees. The `ISOLATE` property sets or suppresses the direction of native bidi marks. The `FOLD` property controls state-aware folds, including setup commands and gutters. Stateless fold queries still use stock rules. The `INDENT` property sets an indentation strategy. A strategy can stop or use an ancestor result, with at most 64 strategy calls per query. Configured bracket matching uses the `CLOSED_BY`, `OPENED_BY`, and `BRACKET_MATCHING_HANDLE` properties. The legacy stock query stays unchanged. Use `style_tags` for WAST highlight rules by node name, parent path, inheritance, and opacity. Raw node queries take the ancestor names in root-to-parent order. Synchronous and published viewport trees select configured root and sublanguage data with `LANGUAGE_DATA` and `SUBLANGUAGES`.

In an installed Markdown parser, use `MarkdownCodeLanguages::Configured` to select `MarkdownCodeParser::Wast` for a named fence or a fallback. The mounted tree retains its root properties and data facet. A strict child parse error rejects creation or the entire transaction at the original document byte offset. GPUI retains this handle for body syntax, text previews, and review. `MarkdownCodeParser::parse` parses a detached editor body with original byte offsets. Applying changes to that snapshot retains its strict or recovery policy. The installed Markdown state still validates the entire GPUI transaction. `WastDocument::foldable_in_scope` checks a contiguous body against the parent state before running node callbacks with parent-state byte offsets. Installed Markdown folds and the GPUI command and gutter use it after editor-wide fold services.

### Experimental native WAST viewport parsing

Install `wast_language().viewport_extension()` instead of the synchronous `extension()`. This opt-in is for a whole-document native `EditorView`. Strict configurations return `None`. The default WAST attachment and its incremental checkpoints do not change.

- The view starts one cold recovery job per accepted text or active attachment change. A new attachment targets the first 3,000 normalized UTF-16 units. An edit targets the greater of the mapped working prefix and the mapped measured viewport end. If the old working tree was complete, the target is the new EOF. The view spends at most 4,096 live rounds during the update. It can publish an initial prefix and restore deeper coverage on idle calls. Input setup can scan and copy the full text. The job does not parse a slice as EOF or retry with another parser.
- After the host reports viewport geometry, it can request an idle ticket and call `run_wast_idle` with a live round quota. Each call uses the current viewport interval. It targets the viewport first if the working tree does not cover it and more than 1,000 normalized UTF-16 units follow it. Otherwise it targets the viewport end plus 100,000 units, clamped to EOF. The targets map to raw UTF-8 bytes and round up within a scalar or CRLF. A quota does not bound time or output work. A round can scan a long token.
- `ensure_wast_syntax_tree` stages output without publishing it. `wast_syntax_tree_available` checks the staged and published coverage without parse work. A view installs the initial publication on its state and updates it as part of accepted text and parser changes. Retained states keep their previous trees. A bare state has no preview parse until a view initializes it. A second view can share an incoming publication, but it never shares a job or idle ticket.
- `force_wast_parsing` and completed idle calls commit a newer publication to the current state. A commit evaluates fields and notifies view plugins and listeners once. It runs no transaction filters or extenders and does not change the text, selection, revision, or history. A rejected commit keeps the previous state and can retry the ready tree without parse rounds. A parse error stops work without a fallback and preserves the last published prefix. Detaching or resetting a view invalidates the old idle tickets.
- Request the whole prefix your query needs with `wast_syntax_tree(upto)` on the state or view. Use `wast_viewport_highlights(range)` for contextual highlights. A token can extend past `parsed_to` without extending coverage. An EOF query requires a complete parse. `EditorState::language_data_at` uses published nodes for configured sublanguage predicates and preserves provider order. Viewport trees remain separate from whole-document syntax for indentation, folds, brackets, and other synchronous consumers, even at EOF.

The host owns callback delivery and repaint. There is no GPUI scheduler, browser input-pending check, wall-clock budget, focus bonus, or fragment reuse on edits. A selection or geometry change does not start a new job.

## CodeMirror 6 reference

The [reference manifest](../../docs/codemirror-upstream.json) pins the npm archives and integrity digests. Run `node tests/fixtures/verify_pinned_modules.mjs` from this crate to check the pinned dependency graph. Pass an archive directory to reuse downloaded files. The first [Rust fixtures](tests/upstream.rs) use independent inputs checked against these releases:

| Package | npm `gitHead` | Upstream tests inspected |
| --- | --- | --- |
| [`@codemirror/state` 6.7.6](https://registry.npmjs.org/@codemirror/state/-/state-6.7.6.tgz) | `651084c8a322b3a19b2bb3e00eee7d4a61d641af` | `test/test-change.ts`, `test/test-selection.ts`, `test/test-state.ts` |
| [`@codemirror/commands` 6.11.2](https://registry.npmjs.org/@codemirror/commands/-/commands-6.11.2.tgz) | `2d2511920a7201ba65e3a9d86e5bd941aef8dc25` | `test/test-history.ts`, `test/test-commands.ts` |

To check the reference, install the two releases in a temporary directory:

```sh
npm install --ignore-scripts --save-exact @codemirror/state@6.7.6 @codemirror/commands@6.11.2
```

Use `ChangeSet.of` and `mapPos` with association `-1` or `+1`, `EditorSelection.create`, and `EditorState.update`. Upstream positions count UTF-16 code units. To convert an upstream position at a character boundary, count the UTF-8 bytes in the document prefix up to that position. Use the old document for input positions and the new document for mapped positions and explicit selections.

Upstream also maps empty ranges with association `-1` by default, regardless of the stored cursor association. Both cores keep touching nonempty ranges separate. The raw Rust change-set constructor rejects overlapping changes. The state change builders rebase overlapping specifications in input order. A change set decoded from JSON retains its wire sections and checks the original byte length before application. Raw Rust edits keep mixed line endings; the configured builders parse string inputs.

The upstream commands `history`, `undo`, and `redo` replay the two adjacent typing edits in the fixture as separate steps when their `Transaction.time` values differ by 500 ms. At 499 ms they form one step. Isolation, nonadjacent edits, an intervening selection update, and paste also separated groups in the cases checked. Rust supports explicit timestamp annotations, timed adjacent grouping, and isolation. By default, GPUI does not send timestamps and sets composition group boundaries directly.
