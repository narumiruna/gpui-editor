//! Native port of the completion sources in `@codemirror/lang-css` 6.3.1.
//! See `docs/licenses/codemirror-lang-css.txt`.

use std::{
    collections::HashSet,
    sync::{Arc, LazyLock},
};

use regex::Regex;

use crate::autocomplete::session::{CompletionItem, SourceResult};

use super::{CssDocument, CssNode, CssNodeKind::*};

// JavaScript's \w in these upstream expressions is ASCII only.
static IDENTIFIER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\A(?:-?[A-Za-z0-9_][A-Za-z0-9_-]*)?\z").unwrap());
static VARIABLE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\A-(?:-[A-Za-z0-9_-]*)?\z").unwrap());

static PROPERTIES: LazyLock<Arc<[CompletionItem]>> =
    LazyLock::new(|| items(PROPERTY_NAMES, "property").collect());
static VALUES: LazyLock<Arc<[CompletionItem]>> = LazyLock::new(|| {
    items(VALUE_KEYWORDS, "keyword")
        .chain(items(VALUE_CONSTANTS, "constant"))
        .collect()
});
static PSEUDOS: LazyLock<Arc<[CompletionItem]>> =
    LazyLock::new(|| items(PSEUDO_CLASSES, "class").collect());
static TAGS: LazyLock<Arc<[CompletionItem]>> = LazyLock::new(|| items(TAG_NAMES, "type").collect());
static AT_RULES: LazyLock<Arc<[CompletionItem]>> =
    LazyLock::new(|| items(AT_RULE_NAMES, "keyword").collect());

/// Return unfiltered CSS syntax completions at a UTF-8 byte position.
///
/// The result uses the query position as its replacement end. Property options
/// use a fixed browser snapshot, not the CSS properties of a live host.
/// Incomplete syntax can produce different results from Lezer recovery.
pub fn css_completion_source(
    doc: &CssDocument,
    position: usize,
    explicit: bool,
) -> Option<SourceResult> {
    complete(doc, position, explicit, true, |node| {
        node.kind() == VariableName
    })
}

/// Build a completion source with a custom predicate for variable nodes.
///
/// This changes variable recognition and collection, not the CSS grammar.
pub fn define_css_completion_source(
    is_variable: impl Fn(CssNode<'_>) -> bool,
) -> impl Fn(&CssDocument, usize, bool) -> Option<SourceResult> {
    move |doc, position, explicit| complete(doc, position, explicit, false, &is_variable)
}

fn complete(
    doc: &CssDocument,
    position: usize,
    explicit: bool,
    recover_custom_properties: bool,
    is_variable: impl Fn(CssNode<'_>) -> bool,
) -> Option<SourceResult> {
    let text = doc.text();
    text.slice(position..position)?;
    let node = resolve(doc.root(), position, true);
    let range = node.range();
    let from = range.start;
    let kind = node.kind();
    let is_dash = kind == Error && text.slice(range.clone()) == Some("-");
    let custom_property = recover_custom_properties && is_recovered_custom_property(doc, node);

    if kind == PropertyName
        || (is_dash || kind == TagName && !custom_property)
            && matches!(resolve(node, range.end, false).kind(), Block | Styles)
    {
        return Some(fixed(from, &PROPERTIES));
    }
    if kind == ValueName {
        return Some(fixed(from, &VALUES));
    }
    if kind == PseudoClassName {
        return Some(fixed(from, &PSEUDOS));
    }
    let variable = custom_property || is_variable(node);
    if variable || (explicit || is_dash) && is_var_arg(doc, node) {
        let from = if variable || is_dash { from } else { position };
        return Some(variables(doc, from, is_variable));
    }
    if kind == BinOp && text.slice(range.clone()) == Some("-") {
        match recover_var_dash(doc, node) {
            Some(VarDash::Isolated) => return Some(variables(doc, from, is_variable)),
            Some(VarDash::Unclosed) => return None,
            None => {}
        }
    }
    if kind == TagName {
        let in_block = std::iter::successors(node.parent(), |node| node.parent())
            .any(|node| node.kind() == Block);
        return Some(fixed(from, if in_block { &PROPERTIES } else { &TAGS }));
    }
    // Local recovery can emit AtKeyword for a bare @. Lezer reports an error.
    if kind == AtKeyword && text.slice(range) != Some("@") {
        return Some(fixed(from, &AT_RULES));
    }
    if !explicit {
        return None;
    }
    let above = resolve(node, position, false);
    let before = above
        .children()
        .rfind(|child| child.range().start < position);
    let follows_colon = before.is_some_and(|child| child.kind() == Colon);
    match above.kind() {
        PseudoClassSelector if follows_colon => Some(fixed(position, &PSEUDOS)),
        RuleSet
            if before.is_some_and(|node| follows_recovered_pseudo(doc, above, node, position)) =>
        {
            Some(fixed(position, &PSEUDOS))
        }
        Declaration if follows_colon => Some(fixed(position, &VALUES)),
        ArgList => Some(fixed(position, &VALUES)),
        Block | Styles => Some(fixed(position, &PROPERTIES)),
        _ => None,
    }
}

// The pinned source first resolves to the left at token ends, then uses a
// strict containment check when it needs the enclosing syntax at a boundary.
fn resolve(mut node: CssNode<'_>, position: usize, include_end: bool) -> CssNode<'_> {
    let contains = |node: CssNode<'_>| {
        let range = node.range();
        range.start < position && (position < range.end || include_end && position == range.end)
    };
    while !contains(node) {
        let Some(parent) = node.parent() else {
            return node;
        };
        node = parent;
    }
    while let Some(child) = node
        .children()
        .take_while(|child| child.range().start < position)
        .find(|&child| contains(child))
    {
        node = child;
    }
    node
}

fn is_var_arg(doc: &CssDocument, mut node: CssNode<'_>) -> bool {
    if matches!(node.kind(), OpenParen | Error) {
        node = node.parent().unwrap_or(node);
    }
    node.kind() == ArgList
        && node
            .parent()
            .and_then(|parent| parent.children().next())
            .is_some_and(|callee| {
                callee.kind() == Callee && doc.text().slice(callee.range()) == Some("var")
            })
}

// The local tree can put an unfinished custom property in a nested RuleSet.
// Recognize its name only at the start of that rule inside a declaration block.
fn is_recovered_custom_property(doc: &CssDocument, node: CssNode<'_>) -> bool {
    node.kind() == TagName
        && doc.text().as_str()[node.range()].starts_with("--")
        && std::iter::successors(node.parent(), |parent| parent.parent())
            .take_while(|parent| parent.range().start == node.range().start)
            .find(|parent| parent.kind() == RuleSet)
            .is_some_and(|rule| rule.parent().is_some_and(|parent| parent.kind() == Block))
}

// After a bare colon the local pseudo node can end before the following
// whitespace. Lezer includes that gap in the pseudo node up to the block.
fn follows_recovered_pseudo(
    doc: &CssDocument,
    rule: CssNode<'_>,
    node: CssNode<'_>,
    position: usize,
) -> bool {
    if node.kind() != PseudoClassSelector {
        return false;
    }
    let mut tail = node.children().rev();
    if !tail
        .next()
        .is_some_and(|error| error.kind() == Error && error.range().is_empty())
    {
        return false;
    }
    let Some(colon) = tail.next().filter(|colon| colon.kind() == Colon) else {
        return false;
    };
    // Local recovery can split :: into two incomplete single-colon nodes.
    if colon
        .range()
        .start
        .checked_sub(1)
        .is_some_and(|before| doc.text().as_str().as_bytes()[before] == b':')
    {
        return false;
    }
    rule.children()
        .find(|child| child.kind() == Block)
        .is_some_and(|block| {
            let start = block.range().start;
            position < start
                && doc
                    .text()
                    .slice(node.range().end..start)
                    .is_some_and(|gap| {
                        gap.bytes()
                            .all(|byte| matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | 0x0c))
                    })
        })
}

enum VarDash {
    Isolated,
    Unclosed,
}

// Local recovery keeps a bare dash in var(...) as BinOp. The upstream
// tree uses a single-character error when the dash is alone before `)`.
fn recover_var_dash(doc: &CssDocument, node: CssNode<'_>) -> Option<VarDash> {
    let list = node
        .parent()
        .filter(|&parent| parent.kind() == ArgList && is_var_arg(doc, parent))?;
    let mut args = list.children().filter(|arg| arg.kind() != Comment);
    // The first item that does not pass take_while is the dash itself.
    let before = args
        .by_ref()
        .take_while(|arg| arg.range().start < node.range().start)
        .last()?;
    if !matches!(before.kind(), OpenParen | Comma) {
        return None;
    }
    if list
        .children()
        .next_back()
        .is_some_and(|end| end.kind() == Error)
    {
        return Some(VarDash::Unclosed);
    }
    (args.next()?.kind() == CloseParen).then_some(VarDash::Isolated)
}

fn variables(
    doc: &CssDocument,
    from: usize,
    is_variable: impl Fn(CssNode<'_>) -> bool,
) -> SourceResult {
    SourceResult::new(from, variable_names(doc, is_variable))
        .with_valid_for(|prefix, _, _| VARIABLE.is_match(prefix))
}

fn variable_names(
    doc: &CssDocument,
    is_variable: impl Fn(CssNode<'_>) -> bool,
) -> Vec<CompletionItem> {
    let mut seen = HashSet::new();
    let mut options = Vec::new();
    for declaration in doc.nodes().filter(|node| node.kind() == Declaration) {
        for (name, next) in declaration.children().zip(declaration.children().skip(1)) {
            if next.kind() == Colon && is_variable(name) {
                let label = &doc.text().as_str()[name.range()];
                if seen.insert(label) {
                    options.push(item(label, "variable"));
                }
            }
        }
    }
    options
}

fn fixed(from: usize, options: &Arc<[CompletionItem]>) -> SourceResult {
    SourceResult::new(from, Arc::clone(options))
        .with_valid_for(|prefix, _, _| IDENTIFIER.is_match(prefix))
}

fn item(label: &str, kind: &str) -> CompletionItem {
    let mut item = CompletionItem::new(label);
    item.kind = Some(kind.to_owned());
    if kind == "property" {
        item.insert = Some(format!("{label}: "));
    }
    item
}

fn items(names: &'static str, kind: &'static str) -> impl Iterator<Item = CompletionItem> {
    names
        .split_ascii_whitespace()
        .map(move |label| item(label, kind))
}

// Generated by querying the pinned source at `a { col|or: red }` in
// Chromium 159.0.8154.6 (Playwright build 1255). Upstream reads the
// browser's `document.body.style`; its npm package has no static list.
const PROPERTY_NAMES: &str = "
accent-color additive-symbols align-content align-items align-self alignment-baseline all
anchor-name anchor-scope animation animation-composition animation-delay animation-direction
animation-duration animation-fill-mode animation-iteration-count animation-name
animation-play-state animation-range animation-range-end animation-range-start
animation-timeline animation-timing-function animation-trigger app-region appearance
ascent-override aspect-ratio backdrop-filter backface-visibility background
background-attachment background-blend-mode background-clip background-color background-image
background-origin background-position background-position-x background-position-y
background-repeat background-size base-palette baseline-shift baseline-source block-size border
border-block border-block-color border-block-end border-block-end-color border-block-end-style
border-block-end-width border-block-start border-block-start-color border-block-start-style
border-block-start-width border-block-style border-block-width border-bottom border-bottom-color
border-bottom-left-radius border-bottom-right-radius border-bottom-style border-bottom-width
border-collapse border-color border-end-end-radius border-end-start-radius border-image
border-image-outset border-image-repeat border-image-slice border-image-source
border-image-width border-inline border-inline-color border-inline-end border-inline-end-color
border-inline-end-style border-inline-end-width border-inline-start border-inline-start-color
border-inline-start-style border-inline-start-width border-inline-style border-inline-width
border-left border-left-color border-left-style border-left-width border-radius border-right
border-right-color border-right-style border-right-width border-shape border-spacing
border-start-end-radius border-start-start-radius border-style border-top border-top-color
border-top-left-radius border-top-right-radius border-top-style border-top-width border-width
bottom box-decoration-break box-shadow box-sizing break-after break-before break-inside
buffered-rendering caption-side caret-animation caret-color caret-shape clear clip clip-path
clip-rule color color-interpolation color-interpolation-filters color-rendering color-scheme
column-count column-fill column-gap column-height column-rule column-rule-break
column-rule-color column-rule-inset column-rule-inset-cap column-rule-inset-cap-end
column-rule-inset-cap-start column-rule-inset-end column-rule-inset-junction
column-rule-inset-junction-end column-rule-inset-junction-start column-rule-inset-start
column-rule-style column-rule-visibility-items column-rule-width column-span column-width
column-wrap columns contain contain-intrinsic-block-size contain-intrinsic-height
contain-intrinsic-inline-size contain-intrinsic-size contain-intrinsic-width container
container-name container-type content content-visibility corner-block-end-shape
corner-block-start-shape corner-bottom-left-shape corner-bottom-right-shape corner-bottom-shape
corner-end-end-shape corner-end-start-shape corner-inline-end-shape corner-inline-start-shape
corner-left-shape corner-right-shape corner-shape corner-start-end-shape
corner-start-start-shape corner-top-left-shape corner-top-right-shape corner-top-shape
counter-increment counter-reset counter-set cursor cx cy d descent-override direction display
dominant-baseline dynamic-range-limit empty-cells fallback field-sizing fill fill-opacity
fill-rule filter flex flex-basis flex-direction flex-flow flex-grow flex-line-count flex-shrink
flex-wrap float flood-color flood-opacity font font-display font-family font-feature-settings
font-kerning font-language-override font-optical-sizing font-palette font-size font-size-adjust
font-stretch font-style font-synthesis font-synthesis-small-caps font-synthesis-style
font-synthesis-weight font-variant font-variant-alternates font-variant-caps
font-variant-east-asian font-variant-emoji font-variant-ligatures font-variant-numeric
font-variant-position font-variation-settings font-weight forced-color-adjust frame-sizing gap
grid grid-area grid-auto-columns grid-auto-flow grid-auto-rows grid-column grid-column-end
grid-column-gap grid-column-start grid-gap grid-row grid-row-end grid-row-gap grid-row-start
grid-template grid-template-areas grid-template-columns grid-template-rows height
hyphenate-character hyphenate-limit-chars hyphens image-orientation image-rendering inherits
initial-letter initial-value inline-size inset inset-block inset-block-end inset-block-start
inset-inline inset-inline-end inset-inline-start interactivity interest-delay interest-delay-end
interest-delay-start interpolate-size isolation justify-content justify-items justify-self left
letter-spacing lighting-color line-break line-gap-override line-height list-style
list-style-image list-style-position list-style-type margin margin-block margin-block-end
margin-block-start margin-bottom margin-inline margin-inline-end margin-inline-start margin-left
margin-right margin-top margin-trim marker marker-end marker-mid marker-start mask mask-clip
mask-composite mask-image mask-mode mask-origin mask-position mask-repeat mask-size mask-type
math-depth math-shift math-style max-block-size max-height max-inline-size max-width
min-block-size min-height min-inline-size min-width mix-blend-mode navigation negative
object-fit object-position object-view-box offset offset-anchor offset-distance offset-path
offset-position offset-rotate opacity order orphans outline outline-color outline-offset
outline-style outline-width overflow overflow-anchor overflow-block overflow-clip-margin
overflow-inline overflow-wrap overflow-x overflow-y overlay override-colors overscroll-behavior
overscroll-behavior-block overscroll-behavior-inline overscroll-behavior-x overscroll-behavior-y
pad padding padding-block padding-block-end padding-block-start padding-bottom padding-inline
padding-inline-end padding-inline-start padding-left padding-right padding-top page
page-break-after page-break-before page-break-inside page-margin-safety page-orientation
paint-order perspective perspective-origin place-content place-items place-self pointer-events
position position-anchor position-area position-try position-try-fallbacks position-try-order
position-visibility prefix print-color-adjust quotes r range reading-flow reading-order resize
result right rotate row-gap row-rule row-rule-break row-rule-color row-rule-inset
row-rule-inset-cap row-rule-inset-cap-end row-rule-inset-cap-start row-rule-inset-end
row-rule-inset-junction row-rule-inset-junction-end row-rule-inset-junction-start
row-rule-inset-start row-rule-style row-rule-visibility-items row-rule-width ruby-align
ruby-overhang ruby-position rule rule-break rule-color rule-inset rule-inset-cap rule-inset-end
rule-inset-junction rule-inset-start rule-overlap rule-style rule-visibility-items rule-width rx
ry scale scroll-axis-lock scroll-behavior scroll-initial-target scroll-margin
scroll-margin-block scroll-margin-block-end scroll-margin-block-start scroll-margin-bottom
scroll-margin-inline scroll-margin-inline-end scroll-margin-inline-start scroll-margin-left
scroll-margin-right scroll-margin-top scroll-marker-group scroll-padding scroll-padding-block
scroll-padding-block-end scroll-padding-block-start scroll-padding-bottom scroll-padding-inline
scroll-padding-inline-end scroll-padding-inline-start scroll-padding-left scroll-padding-right
scroll-padding-top scroll-snap-align scroll-snap-stop scroll-snap-type scroll-target-group
scroll-timeline scroll-timeline-axis scroll-timeline-name scrollbar-color scrollbar-gutter
scrollbar-width shape-image-threshold shape-margin shape-outside shape-rendering size
size-adjust speak speak-as src stop-color stop-opacity stroke stroke-dasharray stroke-dashoffset
stroke-linecap stroke-linejoin stroke-miterlimit stroke-opacity stroke-width suffix symbols
syntax system tab-size table-layout text-align text-align-last text-anchor text-autospace
text-box text-box-edge text-box-trim text-combine-upright text-decoration text-decoration-color
text-decoration-inset text-decoration-line text-decoration-skip-ink text-decoration-style
text-decoration-thickness text-emphasis text-emphasis-color text-emphasis-position
text-emphasis-style text-fit text-indent text-justify text-orientation text-overflow
text-rendering text-shadow text-size-adjust text-spacing-trim text-transform
text-underline-offset text-underline-position text-wrap text-wrap-mode text-wrap-style
timeline-scope timeline-trigger timeline-trigger-activation-range
timeline-trigger-activation-range-end timeline-trigger-activation-range-start
timeline-trigger-active-range timeline-trigger-active-range-end
timeline-trigger-active-range-start timeline-trigger-name timeline-trigger-source top
touch-action transform transform-box transform-origin transform-style transition
transition-behavior transition-delay transition-duration transition-property
transition-timing-function translate trigger-scope types unicode-bidi unicode-range user-select
vector-effect vertical-align view-timeline view-timeline-axis view-timeline-inset
view-timeline-name view-transition-class view-transition-group view-transition-name
view-transition-scope visibility webkit-align-content webkit-align-items webkit-align-self
webkit-animation webkit-animation-delay webkit-animation-direction webkit-animation-duration
webkit-animation-fill-mode webkit-animation-iteration-count webkit-animation-name
webkit-animation-play-state webkit-animation-timing-function webkit-app-region webkit-appearance
webkit-backface-visibility webkit-background-clip webkit-background-origin
webkit-background-size webkit-border-after webkit-border-after-color webkit-border-after-style
webkit-border-after-width webkit-border-before webkit-border-before-color
webkit-border-before-style webkit-border-before-width webkit-border-bottom-left-radius
webkit-border-bottom-right-radius webkit-border-end webkit-border-end-color
webkit-border-end-style webkit-border-end-width webkit-border-horizontal-spacing
webkit-border-image webkit-border-radius webkit-border-start webkit-border-start-color
webkit-border-start-style webkit-border-start-width webkit-border-top-left-radius
webkit-border-top-right-radius webkit-border-vertical-spacing webkit-box-align
webkit-box-decoration-break webkit-box-direction webkit-box-flex webkit-box-ordinal-group
webkit-box-orient webkit-box-pack webkit-box-reflect webkit-box-shadow webkit-box-sizing
webkit-clip-path webkit-column-break-after webkit-column-break-before webkit-column-break-inside
webkit-column-count webkit-column-gap webkit-column-rule webkit-column-rule-color
webkit-column-rule-style webkit-column-rule-width webkit-column-span webkit-column-width
webkit-columns webkit-filter webkit-flex webkit-flex-basis webkit-flex-direction
webkit-flex-flow webkit-flex-grow webkit-flex-shrink webkit-flex-wrap
webkit-font-feature-settings webkit-font-smoothing webkit-hyphenate-character
webkit-justify-content webkit-line-break webkit-line-clamp webkit-locale webkit-logical-height
webkit-logical-width webkit-margin-after webkit-margin-before webkit-margin-end
webkit-margin-start webkit-mask webkit-mask-box-image webkit-mask-box-image-outset
webkit-mask-box-image-repeat webkit-mask-box-image-slice webkit-mask-box-image-source
webkit-mask-box-image-width webkit-mask-clip webkit-mask-composite webkit-mask-image
webkit-mask-origin webkit-mask-position webkit-mask-position-x webkit-mask-position-y
webkit-mask-repeat webkit-mask-size webkit-max-logical-height webkit-max-logical-width
webkit-min-logical-height webkit-min-logical-width webkit-opacity webkit-order
webkit-padding-after webkit-padding-before webkit-padding-end webkit-padding-start
webkit-perspective webkit-perspective-origin webkit-perspective-origin-x
webkit-perspective-origin-y webkit-print-color-adjust webkit-rtl-ordering webkit-ruby-position
webkit-shape-image-threshold webkit-shape-margin webkit-shape-outside webkit-tap-highlight-color
webkit-text-combine webkit-text-decorations-in-effect webkit-text-emphasis
webkit-text-emphasis-color webkit-text-emphasis-position webkit-text-emphasis-style
webkit-text-fill-color webkit-text-orientation webkit-text-security webkit-text-size-adjust
webkit-text-stroke webkit-text-stroke-color webkit-text-stroke-width webkit-transform
webkit-transform-origin webkit-transform-origin-x webkit-transform-origin-y
webkit-transform-origin-z webkit-transform-style webkit-transition webkit-transition-delay
webkit-transition-duration webkit-transition-property webkit-transition-timing-function
webkit-user-drag webkit-user-modify webkit-user-select webkit-writing-mode white-space
white-space-collapse widows width will-change window-drag word-break word-spacing word-wrap
writing-mode x y z-index zoom
";

// Generated from the fixed options of the pinned npm distribution.
// Preserve the original order and label case.
const VALUE_KEYWORDS: &str = "
above absolute activeborder additive activecaption after-white-space ahead alias all all-scroll
alphabetic alternate always antialiased appworkspace asterisks attr auto auto-flow avoid
avoid-column avoid-page avoid-region axis-pan background backwards baseline below bidi-override
blink block block-axis bold bolder border border-box both bottom break break-all break-word
bullets button button-bevel buttonface buttonhighlight buttonshadow buttontext calc capitalize
caps-lock-indicator caption captiontext caret cell center checkbox circle cjk-decimal clear clip
close-quote col-resize collapse color color-burn color-dodge column column-reverse compact
condensed contain content contents content-box context-menu continuous copy counter counters
cover crop cross crosshair currentcolor cursive cyclic darken dashed decimal
decimal-leading-zero default default-button dense destination-atop destination-in
destination-out destination-over difference disc discard disclosure-closed disclosure-open
document dot-dash dot-dot-dash dotted double down e-resize ease ease-in ease-in-out ease-out
element ellipse ellipsis embed end ethiopic-abegede-gez ethiopic-halehame-aa-er
ethiopic-halehame-gez ew-resize exclusion expanded extends extra-condensed extra-expanded
fantasy fast fill fill-box fixed flat flex flex-end flex-start footnotes forwards from
geometricPrecision graytext grid groove hand hard-light help hidden hide higher highlight
highlighttext horizontal hsl hsla hue icon ignore inactiveborder inactivecaption
inactivecaptiontext infinite infobackground infotext inherit initial inline inline-axis
inline-block inline-flex inline-grid inline-table inset inside intrinsic invert italic justify
keep-all landscape large larger left level lighter lighten line-through linear linear-gradient
lines list-item listbox listitem local logical loud lower lower-hexadecimal lower-latin
lower-norwegian lowercase ltr luminosity manipulation match matrix matrix3d medium menu menutext
message-box middle min-intrinsic mix monospace move multiple multiple_mask_images multiply
n-resize narrower ne-resize nesw-resize no-close-quote no-drop no-open-quote no-repeat none
normal not-allowed nowrap ns-resize numbers numeric nw-resize nwse-resize oblique opacity
open-quote optimizeLegibility optimizeSpeed outset outside outside-shape overlay overline
padding padding-box painted page paused perspective pinch-zoom plus-darker plus-lighter pointer
polygon portrait pre pre-line pre-wrap preserve-3d progress push-button radial-gradient radio
read-only read-write read-write-plaintext-only rectangle region relative repeat
repeating-linear-gradient repeating-radial-gradient repeat-x repeat-y reset reverse rgb rgba
ridge right rotate rotate3d rotateX rotateY rotateZ round row row-resize row-reverse rtl run-in
running s-resize sans-serif saturation scale scale3d scaleX scaleY scaleZ screen scroll
scrollbar scroll-position se-resize self-start self-end semi-condensed semi-expanded separate
serif show single skew skewX skewY skip-white-space slide slider-horizontal slider-vertical
sliderthumb-horizontal sliderthumb-vertical slow small small-caps small-caption smaller
soft-light solid source-atop source-in source-out source-over space space-around space-between
space-evenly spell-out square start static status-bar stretch stroke stroke-box sub
subpixel-antialiased svg_masks super sw-resize symbolic symbols system-ui table table-caption
table-cell table-column table-column-group table-footer-group table-header-group table-row
table-row-group text text-bottom text-top textarea textfield thick thin threeddarkshadow
threedface threedhighlight threedlightshadow threedshadow to top transform translate translate3d
translateX translateY translateZ transparent ultra-condensed ultra-expanded underline
unidirectional-pan unset up upper-latin uppercase url var vertical vertical-text view-box
visible visibleFill visiblePainted visibleStroke visual w-resize wait wave wider window
windowframe windowtext words wrap wrap-reverse x-large x-small xor xx-large xx-small
";

const VALUE_CONSTANTS: &str = "
aliceblue antiquewhite aqua aquamarine azure beige bisque black blanchedalmond blue blueviolet
brown burlywood cadetblue chartreuse chocolate coral cornflowerblue cornsilk crimson cyan
darkblue darkcyan darkgoldenrod darkgray darkgreen darkkhaki darkmagenta darkolivegreen
darkorange darkorchid darkred darksalmon darkseagreen darkslateblue darkslategray darkturquoise
darkviolet deeppink deepskyblue dimgray dodgerblue firebrick floralwhite forestgreen fuchsia
gainsboro ghostwhite gold goldenrod gray grey green greenyellow honeydew hotpink indianred
indigo ivory khaki lavender lavenderblush lawngreen lemonchiffon lightblue lightcoral lightcyan
lightgoldenrodyellow lightgray lightgreen lightpink lightsalmon lightseagreen lightskyblue
lightslategray lightsteelblue lightyellow lime limegreen linen magenta maroon mediumaquamarine
mediumblue mediumorchid mediumpurple mediumseagreen mediumslateblue mediumspringgreen
mediumturquoise mediumvioletred midnightblue mintcream mistyrose moccasin navajowhite navy
oldlace olive olivedrab orange orangered orchid palegoldenrod palegreen paleturquoise
palevioletred papayawhip peachpuff peru pink plum powderblue purple rebeccapurple red rosybrown
royalblue saddlebrown salmon sandybrown seagreen seashell sienna silver skyblue slateblue
slategray snow springgreen steelblue tan teal thistle tomato turquoise violet wheat white
whitesmoke yellow yellowgreen
";

const PSEUDO_CLASSES: &str = "
active after any-link autofill backdrop before checked cue default defined disabled empty
enabled file-selector-button first first-child first-letter first-line first-of-type focus
focus-visible focus-within fullscreen has host host-context hover in-range indeterminate invalid
is lang last-child last-of-type left link marker modal not nth-child nth-last-child
nth-last-of-type nth-of-type only-child only-of-type optional out-of-range part placeholder
placeholder-shown read-only read-write required right root scope selection slotted target
target-text valid visited where
";

const TAG_NAMES: &str = "
a abbr address article aside b bdi bdo blockquote body br button canvas caption cite code col
colgroup dd del details dfn dialog div dl dt em figcaption figure footer form header hgroup h1
h2 h3 h4 h5 h6 hr html i iframe img input ins kbd label legend li main meter nav ol output p pre
ruby section select small source span strong sub summary sup table tbody td template textarea
tfoot th thead tr u ul
";

const AT_RULE_NAMES: &str = "
@charset @color-profile @container @counter-style @font-face @font-feature-values
@font-palette-values @import @keyframes @layer @media @namespace @page @position-try @property
@scope @starting-style @supports @view-transition
";
