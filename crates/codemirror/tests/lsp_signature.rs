use std::ops::Range;

use gpui_editor_codemirror::{
    Change, ChangeSet, Text,
    lsp::{
        DocumentWorkspace, LspError, SyncKind,
        features::{Documentation, FeatureError},
        signature::{
            SIGNATURE_KEYMAP, SignatureEffect, SignaturePlan, SignatureRequest, SignatureTrigger,
            signature_request,
        },
    },
};
use serde_json::{Value, json};

const URI: &str = "file:///oracle.js";

fn open(source: &str) -> DocumentWorkspace {
    let mut workspace = DocumentWorkspace::default();
    workspace
        .open_file(URI, "javascript", Text::new(source))
        .unwrap();
    workspace
}

fn change(workspace: &mut DocumentWorkspace, range: Range<usize>, text: &str) {
    workspace
        .record_change(URI, &ChangeSet::single(Change::new(range, text)).unwrap())
        .unwrap();
}

fn assert_display(workspace: &DocumentWorkspace, active: Option<&SignaturePlan>, expected: &Value) {
    let display = active.map(|plan| {
        let count = plan.signatures().len();
        json!({
            "anchor": plan.anchor(workspace).unwrap(),
            "counter": (count > 1).then(|| format!("{}/{}", plan.active_index() + 1, count)),
            "label": plan.signature().label,
            "highlight": plan.highlight().map(|span| [span.start, span.end]),
        })
    });
    assert_eq!(display.unwrap_or(Value::Null), *expected);
}

fn assert_effect(
    workspace: &DocumentWorkspace,
    request: &SignatureRequest,
    active: &mut Option<SignaturePlan>,
    step: &Value,
    effect: &str,
) {
    assert_eq!(*request.params(), step["params"]);
    let actual = match request
        .resolve(workspace, &step["result"], active.as_ref())
        .unwrap()
    {
        SignatureEffect::Keep => "keep",
        SignatureEffect::Clear => {
            *active = None;
            "clear"
        }
        SignatureEffect::Show(plan) => {
            assert_eq!(plan.version(), request.version());
            assert_eq!(plan.uri(), URI);
            *active = Some(plan);
            "show"
        }
    };
    assert_eq!(effect, actual);
    assert_display(workspace, active.as_ref(), &step["display"]);
}

// The fixture records outbound requests and DOM display effects from the
// SRI-verified npm tarball. A scratch jsdom editor used the public commands
// and an in-memory RPC peer. The first reply is from upstream test/server.ts.
// The tests below use the recorded JSON only; they do not need JavaScript.
#[test]
fn pinned_signature_requests_and_display_effects() {
    let oracle: Value = serde_json::from_str(include_str!("fixtures/lsp_signature.json")).unwrap();
    assert_eq!(oracle["package"], "@codemirror/lsp-client@6.3.1");
    let upstream: Value =
        serde_json::from_str(include_str!("../../../docs/codemirror-upstream.json")).unwrap();
    let pinned = upstream["packages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|package| package["name"] == "@codemirror/lsp-client")
        .unwrap();
    assert_eq!(oracle["integrity"], pinned["integrity"]);
    assert_eq!(json!(SIGNATURE_KEYMAP.map(|(key, _)| key)), oracle["keys"]);
    assert_eq!(oracle["uri"], URI);
    let source = oracle["doc"].as_str().unwrap();
    let mut workspace = open(source);
    let mut active = None;
    let request = signature_request(
        &workspace,
        URI,
        source.len(),
        SignatureTrigger::Invoked,
        active.as_ref(),
    )
    .unwrap();
    assert_eq!(request.method(), "textDocument/signatureHelp");
    assert_eq!(request.capability(), "signatureHelpProvider");
    assert_eq!(request.version(), 0);
    assert_effect(
        &workspace,
        &request,
        &mut active,
        &oracle["invoked"],
        "show",
    );
    let first = active.as_ref().unwrap();
    assert_display(
        &workspace,
        Some(&first.prev_signature()),
        &oracle["invoked"]["display"],
    );
    let next = first.next_signature();
    assert_display(&workspace, Some(&next), &oracle["next"]);
    assert_display(&workspace, Some(&next.next_signature()), &oracle["next"]);
    assert_display(
        &workspace,
        Some(&next.prev_signature()),
        &oracle["invoked"]["display"],
    );
    assert_display(&workspace, Some(first), &oracle["invoked"]["display"]);

    let request = signature_request(
        &workspace,
        URI,
        0,
        SignatureTrigger::ContentChange,
        Some(first),
    )
    .unwrap();
    // Navigation after sending the request affects the result, not the wire context.
    active = Some(next);
    assert_effect(
        &workspace,
        &request,
        &mut active,
        &oracle["contentChange"],
        "show",
    );
    let request = signature_request(
        &workspace,
        URI,
        0,
        SignatureTrigger::Invoked,
        active.as_ref(),
    )
    .unwrap();
    assert_effect(
        &workspace,
        &request,
        &mut active,
        &oracle["sameLabels"],
        "keep",
    );

    change(&mut workspace, 0..0, "[,(");
    assert_eq!(
        active.as_ref().unwrap().anchor(&workspace).unwrap(),
        source.len() + 3
    );
    let notifications = workspace.sync(SyncKind::Incremental).unwrap();
    assert_eq!(notifications[0]["params"]["textDocument"]["version"], 1);
    let trigger = SignatureTrigger::automatic(["[,("], true, &oracle["capabilities"]).unwrap();
    let request = signature_request(&workspace, URI, 3, trigger, active.as_ref()).unwrap();
    assert_effect(
        &workspace,
        &request,
        &mut active,
        &oracle["character"],
        "show",
    );
    let first = active.as_ref().unwrap();
    assert_eq!(
        first.signature().documentation,
        Some(Documentation::Markdown(
            "[inert](https://example.invalid)".into()
        ))
    );
    assert_eq!(
        first.active_parameter().unwrap().documentation,
        Some(Documentation::PlainText("second emoji".into()))
    );
    active = Some(first.next_signature());
    assert_display(&workspace, active.as_ref(), &oracle["nextUnicode"]);

    let request = signature_request(
        &workspace,
        URI,
        3,
        SignatureTrigger::Invoked,
        active.as_ref(),
    )
    .unwrap();
    change(&mut workspace, 0..0, "é");
    assert_effect(&workspace, &request, &mut active, &oracle["mapped"], "show");
    assert_eq!(request.version(), 1);
    workspace.sync(SyncKind::Incremental).unwrap();
    let request = signature_request(
        &workspace,
        URI,
        5,
        SignatureTrigger::Invoked,
        active.as_ref(),
    )
    .unwrap();
    assert_eq!(*request.params(), oracle["dropped"]["params"]);
    // The host discards the request when a selection change supersedes it.
    drop(request);
    assert_display(&workspace, active.as_ref(), &oracle["dropped"]["display"]);

    let request = signature_request(
        &workspace,
        URI,
        0,
        SignatureTrigger::Invoked,
        active.as_ref(),
    )
    .unwrap();
    assert_effect(
        &workspace,
        &request,
        &mut active,
        &oracle["cleared"],
        "clear",
    );
    let request = signature_request(
        &workspace,
        URI,
        0,
        SignatureTrigger::Invoked,
        active.as_ref(),
    )
    .unwrap();
    assert_effect(&workspace, &request, &mut active, &oracle["empty"], "keep");
}

#[test]
fn automatic_triggers_follow_configuration_and_document_order() {
    let capabilities = json!({"signatureHelpProvider":{
        "triggerCharacters": ["(", "[", "::"], "retriggerCharacters": [","]
    }});
    let trigger = |character: &str| Some(SignatureTrigger::Character(character.into()));
    assert_eq!(
        SignatureTrigger::automatic(["[,("], false, &capabilities),
        trigger("[")
    );
    assert_eq!(
        SignatureTrigger::automatic(["::[", "(,"], true, &capabilities),
        trigger(",")
    );
    assert_eq!(
        SignatureTrigger::automatic(["::[", "(,"], false, &capabilities),
        trigger("(")
    );
    assert_eq!(
        SignatureTrigger::automatic(["::", "anything"], false, &capabilities),
        trigger("::")
    );
    assert_eq!(
        SignatureTrigger::automatic([","], false, &capabilities),
        None
    );
    assert_eq!(
        SignatureTrigger::automatic(["("], false, &json!({"signatureHelpProvider":false})),
        None
    );
}

#[test]
fn request_and_plan_refuse_a_different_workspace_and_a_reopened_document() {
    let mut workspace = open("😀call(");
    let other = open("😀call(");
    let request = signature_request(&workspace, URI, 4, SignatureTrigger::Invoked, None).unwrap();
    let result = json!({"signatures":[{"label":"call(x)"}]});
    let SignatureEffect::Show(plan) = request.resolve(&workspace, &result, None).unwrap() else {
        panic!("expected a signature plan");
    };
    assert!(matches!(
        request.resolve(&other, &Value::Null, None),
        Err(FeatureError::StaleResponse { .. })
    ));
    assert!(matches!(
        plan.anchor(&other),
        Err(FeatureError::StaleResponse { .. })
    ));
    workspace
        .open_file("file:///other.js", "javascript", Text::new("x"))
        .unwrap();
    assert!(matches!(
        signature_request(
            &workspace,
            "file:///other.js",
            0,
            SignatureTrigger::Invoked,
            Some(&plan)
        ),
        Err(FeatureError::StaleResponse { .. })
    ));
    workspace.close_file(URI).unwrap();
    assert!(matches!(
        request.resolve(&workspace, &result, None),
        Err(FeatureError::StaleResponse { current: None, .. })
    ));
    workspace
        .open_file(URI, "javascript", Text::new("😀call("))
        .unwrap();
    assert!(matches!(
        plan.anchor(&workspace),
        Err(FeatureError::StaleResponse {
            requested: 0,
            current: Some(1),
            ..
        })
    ));
    assert!(matches!(
        signature_request(&workspace, URI, 4, SignatureTrigger::Invoked, Some(&plan)),
        Err(FeatureError::StaleResponse { .. })
    ));
}

#[test]
fn pending_drafts_and_deleted_request_anchors_cannot_be_reused() {
    let mut workspace = open("arg");
    let request = signature_request(&workspace, URI, 1, SignatureTrigger::Invoked, None).unwrap();
    change(&mut workspace, 0..3, "");
    assert!(matches!(
        signature_request(&workspace, URI, 0, SignatureTrigger::Invoked, None),
        Err(FeatureError::UnsyncedDocument(_))
    ));
    let result = json!({"signatures":[{"label":"fn(arg)"}]});
    assert!(matches!(
        request.resolve(&workspace, &result, None),
        Err(FeatureError::StaleResponse {
            requested: 0,
            current: Some(0),
            ..
        })
    ));
}

#[test]
fn invalid_positions_and_signature_payloads_do_not_make_plans() {
    let workspace = open("😀");
    assert!(matches!(
        signature_request(&workspace, URI, 1, SignatureTrigger::Invoked, None),
        Err(FeatureError::Protocol(LspError::InvalidByteOffset(1)))
    ));
    assert!(matches!(
        signature_request(&workspace, URI, 4, SignatureTrigger::ContentChange, None),
        Err(FeatureError::Protocol(LspError::InvalidFormat(
            "SignatureHelpContext"
        )))
    ));
    let request = signature_request(&workspace, URI, 4, SignatureTrigger::Invoked, None).unwrap();
    for result in [
        json!({}),
        json!({"signatures":[{"label":12}]}),
        json!({"signatures":[{"label":"label","documentation":{"kind":"html","value":"markup"}}]}),
        json!({"signatures":[{"label":"😀x","parameters":[{"label":[1,2]}]}]}),
        json!({"signatures":[{"label":"😀x","parameters":[{"label":[3,2]}]}]}),
        json!({"signatures":[{"label":"😀x","parameters":[{"label":[2,4]}]}]}),
        json!({"signatures":[{"label":"😀x","parameters":[{"label":[-1,2]}]}]}),
        json!({"signatures":[{"label":"label"}],"activeParameter":0.5}),
    ] {
        assert!(
            matches!(
                request.resolve(&workspace, &result, None),
                Err(FeatureError::Protocol(LspError::InvalidFormat(_)))
            ),
            "{result}"
        );
    }
}

#[test]
fn an_unmatched_parameter_is_not_highlighted_and_null_clears() {
    let workspace = open("f(");
    let request = signature_request(&workspace, URI, 2, SignatureTrigger::Invoked, None).unwrap();
    let result = json!({
        "signatures":[
            {"label":"f(x)","parameters":[{"label":"missing"}]},
            {"label":"f(y)","activeParameter":2,"parameters":[{"label":"y"}]}
        ],
        "activeSignature":99,"activeParameter":0
    });
    let SignatureEffect::Show(plan) = request.resolve(&workspace, &result, None).unwrap() else {
        panic!("expected a signature plan");
    };
    assert_eq!(plan.active_index(), 0);
    assert_eq!(plan.active_parameter().unwrap().label, "missing");
    assert_eq!(plan.highlight(), None);
    assert_eq!(plan.next_signature().active_parameter(), None);
    assert!(matches!(
        request.resolve(&workspace, &Value::Null, Some(&plan)),
        Ok(SignatureEffect::Clear)
    ));
}
