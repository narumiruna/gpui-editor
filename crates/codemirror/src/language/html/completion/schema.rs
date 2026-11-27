//! Ordered default schema from @codemirror/lang-html 6.4.12.

use super::{HtmlAttributeSpec, HtmlTagSpec};

const TARGETS: &[&str] = &["_blank", "_self", "_top", "_parent"];
const CHARSETS: &[&str] = &["ascii", "utf-8", "utf-16", "latin1", "latin1"];
const METHODS: &[&str] = &["get", "post", "put", "delete"];
const ENCODINGS: &[&str] = &[
    "application/x-www-form-urlencoded",
    "multipart/form-data",
    "text/plain",
];
const BOOL: &[&str] = &["true", "false"];

pub(super) const TAGS: &[HtmlTagSpec<'static>] = &[
    HtmlTagSpec {
        attributes: &[
            HtmlAttributeSpec::new("href", None),
            HtmlAttributeSpec::new("ping", None),
            HtmlAttributeSpec::new("type", None),
            HtmlAttributeSpec::new("media", None),
            HtmlAttributeSpec::new("target", Some(TARGETS)),
            HtmlAttributeSpec::new("hreflang", None),
        ],
        ..HtmlTagSpec::new("a")
    },
    HtmlTagSpec::new("abbr"),
    HtmlTagSpec::new("address"),
    HtmlTagSpec {
        attributes: &[
            HtmlAttributeSpec::new("alt", None),
            HtmlAttributeSpec::new("coords", None),
            HtmlAttributeSpec::new("href", None),
            HtmlAttributeSpec::new("target", None),
            HtmlAttributeSpec::new("ping", None),
            HtmlAttributeSpec::new("media", None),
            HtmlAttributeSpec::new("hreflang", None),
            HtmlAttributeSpec::new("type", None),
            HtmlAttributeSpec::new("shape", Some(&["default", "rect", "circle", "poly"])),
        ],
        ..HtmlTagSpec::new("area")
    },
    HtmlTagSpec::new("article"),
    HtmlTagSpec::new("aside"),
    HtmlTagSpec {
        attributes: &[
            HtmlAttributeSpec::new("src", None),
            HtmlAttributeSpec::new("mediagroup", None),
            HtmlAttributeSpec::new("crossorigin", Some(&["anonymous", "use-credentials"])),
            HtmlAttributeSpec::new("preload", Some(&["none", "metadata", "auto"])),
            HtmlAttributeSpec::new("autoplay", Some(&["autoplay"])),
            HtmlAttributeSpec::new("loop", Some(&["loop"])),
            HtmlAttributeSpec::new("controls", Some(&["controls"])),
        ],
        ..HtmlTagSpec::new("audio")
    },
    HtmlTagSpec::new("b"),
    HtmlTagSpec {
        attributes: &[
            HtmlAttributeSpec::new("href", None),
            HtmlAttributeSpec::new("target", Some(TARGETS)),
        ],
        ..HtmlTagSpec::new("base")
    },
    HtmlTagSpec::new("bdi"),
    HtmlTagSpec::new("bdo"),
    HtmlTagSpec {
        attributes: &[HtmlAttributeSpec::new("cite", None)],
        ..HtmlTagSpec::new("blockquote")
    },
    HtmlTagSpec::new("body"),
    HtmlTagSpec::new("br"),
    HtmlTagSpec {
        attributes: &[
            HtmlAttributeSpec::new("form", None),
            HtmlAttributeSpec::new("formaction", None),
            HtmlAttributeSpec::new("name", None),
            HtmlAttributeSpec::new("value", None),
            HtmlAttributeSpec::new("autofocus", Some(&["autofocus"])),
            HtmlAttributeSpec::new("disabled", Some(&["autofocus"])),
            HtmlAttributeSpec::new("formenctype", Some(ENCODINGS)),
            HtmlAttributeSpec::new("formmethod", Some(METHODS)),
            HtmlAttributeSpec::new("formnovalidate", Some(&["novalidate"])),
            HtmlAttributeSpec::new("formtarget", Some(TARGETS)),
            HtmlAttributeSpec::new("type", Some(&["submit", "reset", "button"])),
        ],
        ..HtmlTagSpec::new("button")
    },
    HtmlTagSpec {
        attributes: &[
            HtmlAttributeSpec::new("width", None),
            HtmlAttributeSpec::new("height", None),
        ],
        ..HtmlTagSpec::new("canvas")
    },
    HtmlTagSpec::new("caption"),
    HtmlTagSpec::new("center"),
    HtmlTagSpec::new("cite"),
    HtmlTagSpec::new("code"),
    HtmlTagSpec {
        attributes: &[HtmlAttributeSpec::new("span", None)],
        ..HtmlTagSpec::new("col")
    },
    HtmlTagSpec {
        attributes: &[HtmlAttributeSpec::new("span", None)],
        ..HtmlTagSpec::new("colgroup")
    },
    HtmlTagSpec {
        attributes: &[
            HtmlAttributeSpec::new("type", Some(&["command", "checkbox", "radio"])),
            HtmlAttributeSpec::new("label", None),
            HtmlAttributeSpec::new("icon", None),
            HtmlAttributeSpec::new("radiogroup", None),
            HtmlAttributeSpec::new("command", None),
            HtmlAttributeSpec::new("title", None),
            HtmlAttributeSpec::new("disabled", Some(&["disabled"])),
            HtmlAttributeSpec::new("checked", Some(&["checked"])),
        ],
        ..HtmlTagSpec::new("command")
    },
    HtmlTagSpec {
        attributes: &[HtmlAttributeSpec::new("value", None)],
        ..HtmlTagSpec::new("data")
    },
    HtmlTagSpec {
        attributes: &[
            HtmlAttributeSpec::new("disabled", Some(&["disabled"])),
            HtmlAttributeSpec::new("multiple", Some(&["multiple"])),
        ],
        ..HtmlTagSpec::new("datagrid")
    },
    HtmlTagSpec {
        attributes: &[HtmlAttributeSpec::new("data", None)],
        ..HtmlTagSpec::new("datalist")
    },
    HtmlTagSpec::new("dd"),
    HtmlTagSpec {
        attributes: &[
            HtmlAttributeSpec::new("cite", None),
            HtmlAttributeSpec::new("datetime", None),
        ],
        ..HtmlTagSpec::new("del")
    },
    HtmlTagSpec {
        attributes: &[HtmlAttributeSpec::new("open", Some(&["open"]))],
        ..HtmlTagSpec::new("details")
    },
    HtmlTagSpec::new("dfn"),
    HtmlTagSpec::new("div"),
    HtmlTagSpec::new("dl"),
    HtmlTagSpec::new("dt"),
    HtmlTagSpec::new("em"),
    HtmlTagSpec {
        attributes: &[
            HtmlAttributeSpec::new("src", None),
            HtmlAttributeSpec::new("type", None),
            HtmlAttributeSpec::new("width", None),
            HtmlAttributeSpec::new("height", None),
        ],
        ..HtmlTagSpec::new("embed")
    },
    HtmlTagSpec {
        attributes: &[HtmlAttributeSpec::new("src", None)],
        ..HtmlTagSpec::new("eventsource")
    },
    HtmlTagSpec {
        attributes: &[
            HtmlAttributeSpec::new("disabled", Some(&["disabled"])),
            HtmlAttributeSpec::new("form", None),
            HtmlAttributeSpec::new("name", None),
        ],
        ..HtmlTagSpec::new("fieldset")
    },
    HtmlTagSpec::new("figcaption"),
    HtmlTagSpec::new("figure"),
    HtmlTagSpec::new("footer"),
    HtmlTagSpec {
        attributes: &[
            HtmlAttributeSpec::new("action", None),
            HtmlAttributeSpec::new("name", None),
            HtmlAttributeSpec::new("accept-charset", Some(CHARSETS)),
            HtmlAttributeSpec::new("autocomplete", Some(&["on", "off"])),
            HtmlAttributeSpec::new("enctype", Some(ENCODINGS)),
            HtmlAttributeSpec::new("method", Some(METHODS)),
            HtmlAttributeSpec::new("novalidate", Some(&["novalidate"])),
            HtmlAttributeSpec::new("target", Some(TARGETS)),
        ],
        ..HtmlTagSpec::new("form")
    },
    HtmlTagSpec::new("h1"),
    HtmlTagSpec::new("h2"),
    HtmlTagSpec::new("h3"),
    HtmlTagSpec::new("h4"),
    HtmlTagSpec::new("h5"),
    HtmlTagSpec::new("h6"),
    HtmlTagSpec {
        children: Some(&[
            "title", "base", "link", "style", "meta", "script", "noscript", "command",
        ]),
        ..HtmlTagSpec::new("head")
    },
    HtmlTagSpec::new("header"),
    HtmlTagSpec::new("hgroup"),
    HtmlTagSpec::new("hr"),
    HtmlTagSpec {
        attributes: &[HtmlAttributeSpec::new("manifest", None)],
        ..HtmlTagSpec::new("html")
    },
    HtmlTagSpec::new("i"),
    HtmlTagSpec {
        attributes: &[
            HtmlAttributeSpec::new("src", None),
            HtmlAttributeSpec::new("srcdoc", None),
            HtmlAttributeSpec::new("name", None),
            HtmlAttributeSpec::new("width", None),
            HtmlAttributeSpec::new("height", None),
            HtmlAttributeSpec::new(
                "sandbox",
                Some(&[
                    "allow-top-navigation",
                    "allow-same-origin",
                    "allow-forms",
                    "allow-scripts",
                ]),
            ),
            HtmlAttributeSpec::new("seamless", Some(&["seamless"])),
        ],
        ..HtmlTagSpec::new("iframe")
    },
    HtmlTagSpec {
        attributes: &[
            HtmlAttributeSpec::new("alt", None),
            HtmlAttributeSpec::new("src", None),
            HtmlAttributeSpec::new("ismap", None),
            HtmlAttributeSpec::new("usemap", None),
            HtmlAttributeSpec::new("width", None),
            HtmlAttributeSpec::new("height", None),
            HtmlAttributeSpec::new("crossorigin", Some(&["anonymous", "use-credentials"])),
        ],
        ..HtmlTagSpec::new("img")
    },
    HtmlTagSpec {
        attributes: &[
            HtmlAttributeSpec::new("alt", None),
            HtmlAttributeSpec::new("dirname", None),
            HtmlAttributeSpec::new("form", None),
            HtmlAttributeSpec::new("formaction", None),
            HtmlAttributeSpec::new("height", None),
            HtmlAttributeSpec::new("list", None),
            HtmlAttributeSpec::new("max", None),
            HtmlAttributeSpec::new("maxlength", None),
            HtmlAttributeSpec::new("min", None),
            HtmlAttributeSpec::new("name", None),
            HtmlAttributeSpec::new("pattern", None),
            HtmlAttributeSpec::new("placeholder", None),
            HtmlAttributeSpec::new("size", None),
            HtmlAttributeSpec::new("src", None),
            HtmlAttributeSpec::new("step", None),
            HtmlAttributeSpec::new("value", None),
            HtmlAttributeSpec::new("width", None),
            HtmlAttributeSpec::new("accept", Some(&["audio/*", "video/*", "image/*"])),
            HtmlAttributeSpec::new("autocomplete", Some(&["on", "off"])),
            HtmlAttributeSpec::new("autofocus", Some(&["autofocus"])),
            HtmlAttributeSpec::new("checked", Some(&["checked"])),
            HtmlAttributeSpec::new("disabled", Some(&["disabled"])),
            HtmlAttributeSpec::new("formenctype", Some(ENCODINGS)),
            HtmlAttributeSpec::new("formmethod", Some(METHODS)),
            HtmlAttributeSpec::new("formnovalidate", Some(&["novalidate"])),
            HtmlAttributeSpec::new("formtarget", Some(TARGETS)),
            HtmlAttributeSpec::new("multiple", Some(&["multiple"])),
            HtmlAttributeSpec::new("readonly", Some(&["readonly"])),
            HtmlAttributeSpec::new("required", Some(&["required"])),
            HtmlAttributeSpec::new(
                "type",
                Some(&[
                    "hidden",
                    "text",
                    "search",
                    "tel",
                    "url",
                    "email",
                    "password",
                    "datetime",
                    "date",
                    "month",
                    "week",
                    "time",
                    "datetime-local",
                    "number",
                    "range",
                    "color",
                    "checkbox",
                    "radio",
                    "file",
                    "submit",
                    "image",
                    "reset",
                    "button",
                ]),
            ),
        ],
        ..HtmlTagSpec::new("input")
    },
    HtmlTagSpec {
        attributes: &[
            HtmlAttributeSpec::new("cite", None),
            HtmlAttributeSpec::new("datetime", None),
        ],
        ..HtmlTagSpec::new("ins")
    },
    HtmlTagSpec::new("kbd"),
    HtmlTagSpec {
        attributes: &[
            HtmlAttributeSpec::new("challenge", None),
            HtmlAttributeSpec::new("form", None),
            HtmlAttributeSpec::new("name", None),
            HtmlAttributeSpec::new("autofocus", Some(&["autofocus"])),
            HtmlAttributeSpec::new("disabled", Some(&["disabled"])),
            HtmlAttributeSpec::new("keytype", Some(&["RSA"])),
        ],
        ..HtmlTagSpec::new("keygen")
    },
    HtmlTagSpec {
        attributes: &[
            HtmlAttributeSpec::new("for", None),
            HtmlAttributeSpec::new("form", None),
        ],
        ..HtmlTagSpec::new("label")
    },
    HtmlTagSpec::new("legend"),
    HtmlTagSpec {
        attributes: &[HtmlAttributeSpec::new("value", None)],
        ..HtmlTagSpec::new("li")
    },
    HtmlTagSpec {
        attributes: &[
            HtmlAttributeSpec::new("href", None),
            HtmlAttributeSpec::new("type", None),
            HtmlAttributeSpec::new("hreflang", None),
            HtmlAttributeSpec::new("media", None),
            HtmlAttributeSpec::new(
                "sizes",
                Some(&["all", "16x16", "16x16 32x32", "16x16 32x32 64x64"]),
            ),
        ],
        ..HtmlTagSpec::new("link")
    },
    HtmlTagSpec {
        attributes: &[HtmlAttributeSpec::new("name", None)],
        ..HtmlTagSpec::new("map")
    },
    HtmlTagSpec::new("mark"),
    HtmlTagSpec {
        attributes: &[
            HtmlAttributeSpec::new("label", None),
            HtmlAttributeSpec::new("type", Some(&["list", "context", "toolbar"])),
        ],
        ..HtmlTagSpec::new("menu")
    },
    HtmlTagSpec {
        attributes: &[
            HtmlAttributeSpec::new("content", None),
            HtmlAttributeSpec::new("charset", Some(CHARSETS)),
            HtmlAttributeSpec::new(
                "name",
                Some(&[
                    "viewport",
                    "application-name",
                    "author",
                    "description",
                    "generator",
                    "keywords",
                ]),
            ),
            HtmlAttributeSpec::new(
                "http-equiv",
                Some(&[
                    "content-language",
                    "content-type",
                    "default-style",
                    "refresh",
                ]),
            ),
        ],
        ..HtmlTagSpec::new("meta")
    },
    HtmlTagSpec {
        attributes: &[
            HtmlAttributeSpec::new("value", None),
            HtmlAttributeSpec::new("min", None),
            HtmlAttributeSpec::new("low", None),
            HtmlAttributeSpec::new("high", None),
            HtmlAttributeSpec::new("max", None),
            HtmlAttributeSpec::new("optimum", None),
        ],
        ..HtmlTagSpec::new("meter")
    },
    HtmlTagSpec::new("nav"),
    HtmlTagSpec::new("noscript"),
    HtmlTagSpec {
        attributes: &[
            HtmlAttributeSpec::new("data", None),
            HtmlAttributeSpec::new("type", None),
            HtmlAttributeSpec::new("name", None),
            HtmlAttributeSpec::new("usemap", None),
            HtmlAttributeSpec::new("form", None),
            HtmlAttributeSpec::new("width", None),
            HtmlAttributeSpec::new("height", None),
            HtmlAttributeSpec::new("typemustmatch", Some(&["typemustmatch"])),
        ],
        ..HtmlTagSpec::new("object")
    },
    HtmlTagSpec {
        attributes: &[
            HtmlAttributeSpec::new("reversed", Some(&["reversed"])),
            HtmlAttributeSpec::new("start", None),
            HtmlAttributeSpec::new("type", Some(&["1", "a", "A", "i", "I"])),
        ],
        children: Some(&["li", "script", "template", "ul", "ol"]),
        ..HtmlTagSpec::new("ol")
    },
    HtmlTagSpec {
        attributes: &[
            HtmlAttributeSpec::new("disabled", Some(&["disabled"])),
            HtmlAttributeSpec::new("label", None),
        ],
        ..HtmlTagSpec::new("optgroup")
    },
    HtmlTagSpec {
        attributes: &[
            HtmlAttributeSpec::new("disabled", Some(&["disabled"])),
            HtmlAttributeSpec::new("label", None),
            HtmlAttributeSpec::new("selected", Some(&["selected"])),
            HtmlAttributeSpec::new("value", None),
        ],
        ..HtmlTagSpec::new("option")
    },
    HtmlTagSpec {
        attributes: &[
            HtmlAttributeSpec::new("for", None),
            HtmlAttributeSpec::new("form", None),
            HtmlAttributeSpec::new("name", None),
        ],
        ..HtmlTagSpec::new("output")
    },
    HtmlTagSpec::new("p"),
    HtmlTagSpec {
        attributes: &[
            HtmlAttributeSpec::new("name", None),
            HtmlAttributeSpec::new("value", None),
        ],
        ..HtmlTagSpec::new("param")
    },
    HtmlTagSpec::new("pre"),
    HtmlTagSpec {
        attributes: &[
            HtmlAttributeSpec::new("value", None),
            HtmlAttributeSpec::new("max", None),
        ],
        ..HtmlTagSpec::new("progress")
    },
    HtmlTagSpec {
        attributes: &[HtmlAttributeSpec::new("cite", None)],
        ..HtmlTagSpec::new("q")
    },
    HtmlTagSpec::new("rp"),
    HtmlTagSpec::new("rt"),
    HtmlTagSpec::new("ruby"),
    HtmlTagSpec::new("samp"),
    HtmlTagSpec {
        attributes: &[
            HtmlAttributeSpec::new("type", Some(&["text/javascript"])),
            HtmlAttributeSpec::new("src", None),
            HtmlAttributeSpec::new("async", Some(&["async"])),
            HtmlAttributeSpec::new("defer", Some(&["defer"])),
            HtmlAttributeSpec::new("charset", Some(CHARSETS)),
        ],
        ..HtmlTagSpec::new("script")
    },
    HtmlTagSpec::new("section"),
    HtmlTagSpec {
        attributes: &[
            HtmlAttributeSpec::new("form", None),
            HtmlAttributeSpec::new("name", None),
            HtmlAttributeSpec::new("size", None),
            HtmlAttributeSpec::new("autofocus", Some(&["autofocus"])),
            HtmlAttributeSpec::new("disabled", Some(&["disabled"])),
            HtmlAttributeSpec::new("multiple", Some(&["multiple"])),
        ],
        ..HtmlTagSpec::new("select")
    },
    HtmlTagSpec {
        attributes: &[HtmlAttributeSpec::new("name", None)],
        ..HtmlTagSpec::new("slot")
    },
    HtmlTagSpec::new("small"),
    HtmlTagSpec {
        attributes: &[
            HtmlAttributeSpec::new("src", None),
            HtmlAttributeSpec::new("type", None),
            HtmlAttributeSpec::new("media", None),
        ],
        ..HtmlTagSpec::new("source")
    },
    HtmlTagSpec::new("span"),
    HtmlTagSpec::new("strong"),
    HtmlTagSpec {
        attributes: &[
            HtmlAttributeSpec::new("type", Some(&["text/css"])),
            HtmlAttributeSpec::new("media", None),
            HtmlAttributeSpec::new("scoped", None),
        ],
        ..HtmlTagSpec::new("style")
    },
    HtmlTagSpec::new("sub"),
    HtmlTagSpec::new("summary"),
    HtmlTagSpec::new("sup"),
    HtmlTagSpec::new("table"),
    HtmlTagSpec::new("tbody"),
    HtmlTagSpec {
        attributes: &[
            HtmlAttributeSpec::new("colspan", None),
            HtmlAttributeSpec::new("rowspan", None),
            HtmlAttributeSpec::new("headers", None),
        ],
        ..HtmlTagSpec::new("td")
    },
    HtmlTagSpec::new("template"),
    HtmlTagSpec {
        attributes: &[
            HtmlAttributeSpec::new("dirname", None),
            HtmlAttributeSpec::new("form", None),
            HtmlAttributeSpec::new("maxlength", None),
            HtmlAttributeSpec::new("name", None),
            HtmlAttributeSpec::new("placeholder", None),
            HtmlAttributeSpec::new("rows", None),
            HtmlAttributeSpec::new("cols", None),
            HtmlAttributeSpec::new("autofocus", Some(&["autofocus"])),
            HtmlAttributeSpec::new("disabled", Some(&["disabled"])),
            HtmlAttributeSpec::new("readonly", Some(&["readonly"])),
            HtmlAttributeSpec::new("required", Some(&["required"])),
            HtmlAttributeSpec::new("wrap", Some(&["soft", "hard"])),
        ],
        ..HtmlTagSpec::new("textarea")
    },
    HtmlTagSpec::new("tfoot"),
    HtmlTagSpec {
        attributes: &[
            HtmlAttributeSpec::new("colspan", None),
            HtmlAttributeSpec::new("rowspan", None),
            HtmlAttributeSpec::new("headers", None),
            HtmlAttributeSpec::new("scope", Some(&["row", "col", "rowgroup", "colgroup"])),
        ],
        ..HtmlTagSpec::new("th")
    },
    HtmlTagSpec::new("thead"),
    HtmlTagSpec {
        attributes: &[HtmlAttributeSpec::new("datetime", None)],
        ..HtmlTagSpec::new("time")
    },
    HtmlTagSpec::new("title"),
    HtmlTagSpec::new("tr"),
    HtmlTagSpec {
        attributes: &[
            HtmlAttributeSpec::new("src", None),
            HtmlAttributeSpec::new("label", None),
            HtmlAttributeSpec::new("default", None),
            HtmlAttributeSpec::new(
                "kind",
                Some(&[
                    "subtitles",
                    "captions",
                    "descriptions",
                    "chapters",
                    "metadata",
                ]),
            ),
            HtmlAttributeSpec::new("srclang", None),
        ],
        ..HtmlTagSpec::new("track")
    },
    HtmlTagSpec {
        children: Some(&["li", "script", "template", "ul", "ol"]),
        ..HtmlTagSpec::new("ul")
    },
    HtmlTagSpec::new("var"),
    HtmlTagSpec {
        attributes: &[
            HtmlAttributeSpec::new("src", None),
            HtmlAttributeSpec::new("poster", None),
            HtmlAttributeSpec::new("width", None),
            HtmlAttributeSpec::new("height", None),
            HtmlAttributeSpec::new("crossorigin", Some(&["anonymous", "use-credentials"])),
            HtmlAttributeSpec::new("preload", Some(&["auto", "metadata", "none"])),
            HtmlAttributeSpec::new("autoplay", Some(&["autoplay"])),
            HtmlAttributeSpec::new("mediagroup", Some(&["movie"])),
            HtmlAttributeSpec::new("muted", Some(&["muted"])),
            HtmlAttributeSpec::new("controls", Some(&["controls"])),
        ],
        ..HtmlTagSpec::new("video")
    },
    HtmlTagSpec::new("wbr"),
];

pub(super) const GLOBAL_ATTRIBUTES: &[HtmlAttributeSpec<'static>] = &[
    HtmlAttributeSpec::new("accesskey", None),
    HtmlAttributeSpec::new("class", None),
    HtmlAttributeSpec::new("contenteditable", Some(BOOL)),
    HtmlAttributeSpec::new("contextmenu", None),
    HtmlAttributeSpec::new("dir", Some(&["ltr", "rtl", "auto"])),
    HtmlAttributeSpec::new("draggable", Some(&["true", "false", "auto"])),
    HtmlAttributeSpec::new(
        "dropzone",
        Some(&["copy", "move", "link", "string:", "file:"]),
    ),
    HtmlAttributeSpec::new("hidden", Some(&["hidden"])),
    HtmlAttributeSpec::new("id", None),
    HtmlAttributeSpec::new("inert", Some(&["inert"])),
    HtmlAttributeSpec::new("itemid", None),
    HtmlAttributeSpec::new("itemprop", None),
    HtmlAttributeSpec::new("itemref", None),
    HtmlAttributeSpec::new("itemscope", Some(&["itemscope"])),
    HtmlAttributeSpec::new("itemtype", None),
    HtmlAttributeSpec::new(
        "lang",
        Some(&[
            "ar", "bn", "de", "en-GB", "en-US", "es", "fr", "hi", "id", "ja", "pa", "pt", "ru",
            "tr", "zh",
        ]),
    ),
    HtmlAttributeSpec::new("spellcheck", Some(BOOL)),
    HtmlAttributeSpec::new("autocorrect", Some(BOOL)),
    HtmlAttributeSpec::new("autocapitalize", Some(BOOL)),
    HtmlAttributeSpec::new("style", None),
    HtmlAttributeSpec::new("tabindex", None),
    HtmlAttributeSpec::new("title", None),
    HtmlAttributeSpec::new("translate", Some(&["yes", "no"])),
    HtmlAttributeSpec::new(
        "rel",
        Some(&[
            "stylesheet",
            "alternate",
            "author",
            "bookmark",
            "help",
            "license",
            "next",
            "nofollow",
            "noreferrer",
            "prefetch",
            "prev",
            "search",
            "tag",
        ]),
    ),
    HtmlAttributeSpec::new(
        "role",
        Some(&[
            "alert",
            "application",
            "article",
            "banner",
            "button",
            "cell",
            "checkbox",
            "complementary",
            "contentinfo",
            "dialog",
            "document",
            "feed",
            "figure",
            "form",
            "grid",
            "gridcell",
            "heading",
            "img",
            "list",
            "listbox",
            "listitem",
            "main",
            "navigation",
            "region",
            "row",
            "rowgroup",
            "search",
            "switch",
            "tab",
            "table",
            "tabpanel",
            "textbox",
            "timer",
        ]),
    ),
    HtmlAttributeSpec::new("aria-activedescendant", None),
    HtmlAttributeSpec::new("aria-atomic", Some(BOOL)),
    HtmlAttributeSpec::new(
        "aria-autocomplete",
        Some(&["inline", "list", "both", "none"]),
    ),
    HtmlAttributeSpec::new("aria-busy", Some(BOOL)),
    HtmlAttributeSpec::new(
        "aria-checked",
        Some(&["true", "false", "mixed", "undefined"]),
    ),
    HtmlAttributeSpec::new("aria-controls", None),
    HtmlAttributeSpec::new("aria-describedby", None),
    HtmlAttributeSpec::new("aria-disabled", Some(BOOL)),
    HtmlAttributeSpec::new("aria-dropeffect", None),
    HtmlAttributeSpec::new("aria-expanded", Some(&["true", "false", "undefined"])),
    HtmlAttributeSpec::new("aria-flowto", None),
    HtmlAttributeSpec::new("aria-grabbed", Some(&["true", "false", "undefined"])),
    HtmlAttributeSpec::new("aria-haspopup", Some(BOOL)),
    HtmlAttributeSpec::new("aria-hidden", Some(BOOL)),
    HtmlAttributeSpec::new(
        "aria-invalid",
        Some(&["true", "false", "grammar", "spelling"]),
    ),
    HtmlAttributeSpec::new("aria-label", None),
    HtmlAttributeSpec::new("aria-labelledby", None),
    HtmlAttributeSpec::new("aria-level", None),
    HtmlAttributeSpec::new("aria-live", Some(&["off", "polite", "assertive"])),
    HtmlAttributeSpec::new("aria-multiline", Some(BOOL)),
    HtmlAttributeSpec::new("aria-multiselectable", Some(BOOL)),
    HtmlAttributeSpec::new("aria-owns", None),
    HtmlAttributeSpec::new("aria-posinset", None),
    HtmlAttributeSpec::new(
        "aria-pressed",
        Some(&["true", "false", "mixed", "undefined"]),
    ),
    HtmlAttributeSpec::new("aria-readonly", Some(BOOL)),
    HtmlAttributeSpec::new("aria-relevant", None),
    HtmlAttributeSpec::new("aria-required", Some(BOOL)),
    HtmlAttributeSpec::new("aria-selected", Some(&["true", "false", "undefined"])),
    HtmlAttributeSpec::new("aria-setsize", None),
    HtmlAttributeSpec::new(
        "aria-sort",
        Some(&["ascending", "descending", "none", "other"]),
    ),
    HtmlAttributeSpec::new("aria-valuemax", None),
    HtmlAttributeSpec::new("aria-valuemin", None),
    HtmlAttributeSpec::new("aria-valuenow", None),
    HtmlAttributeSpec::new("aria-valuetext", None),
    HtmlAttributeSpec::new("onbeforeunload", None),
    HtmlAttributeSpec::new("oncopy", None),
    HtmlAttributeSpec::new("oncut", None),
    HtmlAttributeSpec::new("ondragstart", None),
    HtmlAttributeSpec::new("ondragover", None),
    HtmlAttributeSpec::new("ondragleave", None),
    HtmlAttributeSpec::new("ondragenter", None),
    HtmlAttributeSpec::new("ondragend", None),
    HtmlAttributeSpec::new("ondrag", None),
    HtmlAttributeSpec::new("onpaste", None),
    HtmlAttributeSpec::new("onfocus", None),
    HtmlAttributeSpec::new("onblur", None),
    HtmlAttributeSpec::new("onchange", None),
    HtmlAttributeSpec::new("onclick", None),
    HtmlAttributeSpec::new("onload", None),
    HtmlAttributeSpec::new("onmousedown", None),
    HtmlAttributeSpec::new("onmouseenter", None),
    HtmlAttributeSpec::new("onmouseleave", None),
    HtmlAttributeSpec::new("onmouseup", None),
    HtmlAttributeSpec::new("onkeydown", None),
    HtmlAttributeSpec::new("onkeyup", None),
    HtmlAttributeSpec::new("onresize", None),
    HtmlAttributeSpec::new("onscroll", None),
    HtmlAttributeSpec::new("onunload", None),
];
