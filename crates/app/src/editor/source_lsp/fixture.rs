//! Std-only LSP peer. The GPUI integration tests compile it into a scratch directory.

use std::{
    fs,
    io::{self, BufRead, Write},
    panic,
};

const URI: &str = "file:///explicit.md";
const SOURCE: &str =
    "😀 id\r\n\r\nTail. [site](https://example.invalid)\r\n\r\n```rs\r\nlet x = 1;\r\n```\r\n";

fn read_frame(input: &mut impl BufRead) -> String {
    let mut header = String::new();
    input.read_line(&mut header).unwrap();
    let size: usize = header
        .strip_prefix("Content-Length: ")
        .unwrap()
        .trim_end()
        .parse()
        .unwrap();
    header.clear();
    input.read_line(&mut header).unwrap();
    assert_eq!(&header, "\r\n");
    let mut bytes = vec![0; size];
    input.read_exact(&mut bytes).unwrap();
    String::from_utf8(bytes).unwrap()
}

fn expect(input: &mut impl BufRead, method: &str) -> String {
    let frame = read_frame(input);
    assert!(
        frame.contains(&format!(r#""method":"{method}""#)),
        "{frame}"
    );
    frame
}

fn write_frame(output: &mut impl Write, body: &str) {
    write!(output, "Content-Length: {}\r\n\r\n{body}", body.len()).unwrap();
    output.flush().unwrap();
}

fn result(output: &mut impl Write, id: i32, body: &str) {
    write_frame(
        output,
        &format!(r#"{{"jsonrpc":"2.0","id":{id},"result":{body}}}"#),
    );
}

fn id(frame: &str) -> i32 {
    frame
        .split_once(r#""id":"#)
        .unwrap()
        .1
        .split([',', '}'])
        .next()
        .unwrap()
        .parse()
        .unwrap()
}

fn change(input: &mut impl BufRead, text: &str, version: i32) {
    let change = expect(input, "textDocument/didChange");
    assert!(
        change.contains(&format!(r#""version":{version}"#)),
        "{change}"
    );
    assert!(change.contains(&format!(r#""uri":"{URI}""#)), "{change}");
    assert!(
        change.contains(&format!(r#""contentChanges":[{{"text":{text:?}}}]"#)),
        "{change}"
    );
}

fn hover(input: &mut impl BufRead, line: u32, character: u32) -> i32 {
    let hover = expect(input, "textDocument/hover");
    assert!(hover.contains(&format!(r#""uri":"{URI}""#)), "{hover}");
    assert!(hover.contains(r#""position":{"#), "{hover}");
    assert!(hover.contains(&format!(r#""line":{line}"#)), "{hover}");
    assert!(
        hover.contains(&format!(r#""character":{character}"#)),
        "{hover}"
    );
    id(&hover)
}

fn flow(input: &mut impl BufRead, output: &mut impl Write) {
    change(input, &SOURCE.replacen("id", "name", 1), 1);
    let first = hover(input, 0, 3);
    result(output, first + 500, r#"{"contents":"unmatched"}"#);
    result(
        output,
        first,
        r#"{"contents":["[link](https://example.invalid) <script>ignored</script>",{"language":"rust","value":"let x = 1;"}],"range":{"start":{"line":0,"character":3},"end":{"line":0,"character":7}}}"#,
    );

    let superseded = hover(input, 0, 4);
    result(output, superseded, r#"{"contents":"old position"}"#);
    let invalid_range = hover(input, 2, 0);
    result(
        output,
        invalid_range,
        r#"{"contents":"invalid range","range":{"start":{"line":0,"character":1},"end":{"line":2,"character":1}}}"#,
    );

    let older_revision = hover(input, 0, 3);
    change(input, SOURCE, 2);
    result(output, older_revision, r#"{"contents":"old revision"}"#);
    hover(input, 0, 3);
}

fn references(input: &mut impl BufRead, line: u32, character: u32) -> i32 {
    let frame = expect(input, "textDocument/references");
    for field in [
        format!(r#""uri":"{URI}""#),
        format!(r#""position":{{"line":{line},"character":{character}}}"#),
        r#""context":{"includeDeclaration":true}"#.into(),
    ] {
        assert!(frame.contains(&field), "{frame}");
    }
    id(&frame)
}

fn location(uri: &str, line: u32, start: u32, end: u32) -> String {
    format!(
        r#"{{"uri":"{uri}","range":{{"start":{{"line":{line},"character":{start}}},"end":{{"line":{line},"character":{end}}}}}}}"#
    )
}

fn references_flow(input: &mut impl BufRead, output: &mut impl Write) {
    change(input, &SOURCE.replacen("id", "name", 1), 1);
    let tail = location(URI, 2, 0, 4);
    let name = location(URI, 0, 3, 7);
    let unknown = location("file:///untrusted.md", 0, 0, 1);
    let link = location(URI, 2, 6, 11);
    let preview = location(URI, 5, 0, 3);
    let mixed = format!("[{unknown},{tail},{name},{tail},{link},{preview}]");
    let first = references(input, 0, 4);
    result(output, first + 500, "null");
    result(output, first, &mixed);
    for body in [
        "null".into(),
        "[]".into(),
        format!("[{unknown}]"),
        format!("[{}]", location(URI, 0, 1, 3)), // Inside a surrogate pair.
        format!("[{}]", [tail.as_str(); 65].join(",")),
    ] {
        result(output, references(input, 0, 4), &body);
    }
    result(output, references(input, 0, 4), &mixed);
    let superseded = references(input, 0, 4);
    result(output, superseded, &format!("[{name}]"));
    let newer = references(input, 2, 0);
    result(output, newer, &format!("[{tail}]"));
    let dismissed = references(input, 2, 0);
    result(output, dismissed, &mixed);
    let stale = references(input, 2, 0);
    change(input, SOURCE, 2);
    result(output, stale, &mixed);
    result(output, references(input, 2, 0), &format!("[{tail}]"));
}

const SIGNATURE: &str = r#"{"signatures":[{"label":"f(😀, x)","parameters":[{"label":[2,4]}],"documentation":{"kind":"markdown","value":"[site](https://example.invalid) <script>literal</script>\u202e"}},{"label":"f(value)"}],"activeParameter":0,"command":{"command":"do not run"},"edit":{"changes":{}},"targetUri":"file:///do-not-open"}"#;

fn signature(
    input: &mut impl BufRead,
    line: u32,
    character: u32,
    trigger: Option<(&str, bool)>,
) -> i32 {
    let frame = expect(input, "textDocument/signatureHelp");
    for field in [
        format!(r#""uri":"{URI}""#),
        format!(r#""position":{{"line":{line},"character":{character}}}"#),
        format!(r#""triggerKind":{}"#, if trigger.is_some() { 2 } else { 1 }),
        format!(
            r#""isRetrigger":{}"#,
            trigger.is_some_and(|(_, active)| active)
        ),
    ] {
        assert!(frame.contains(&field), "{frame}");
    }
    if let Some((character, active)) = trigger {
        assert!(
            frame.contains(&format!(r#""triggerCharacter":"{character}""#)),
            "{frame}"
        );
        assert_eq!(
            frame.contains(r#""activeSignatureHelp":{"#),
            active,
            "{frame}"
        );
        if active {
            assert!(frame.contains(r#""label":"f(x)""#), "{frame}");
        }
    }
    id(&frame)
}

fn signature_request_flow(input: &mut impl BufRead, output: &mut impl Write) {
    let first = signature(input, 0, 3, None);
    result(
        output,
        first + 500,
        r#"{"signatures":[{"label":"unmatched"}]}"#,
    );
    result(output, first, SIGNATURE);
    let moved = signature(input, 0, 3, None);
    result(output, moved, SIGNATURE);
    let stale = signature(input, 0, 3, None);
    change(input, &SOURCE.replacen("Tail.", "Changed.", 1), 1);
    result(output, stale, SIGNATURE);
    let body = signature(input, 5, 0, None);
    result(output, body, r#"{"signatures":[{"label":"in code"}]}"#);
}

fn signature_trigger_flow(input: &mut impl BufRead, output: &mut impl Write) {
    change(input, &SOURCE.replacen(" id", " ,id", 1), 1);
    change(input, &SOURCE.replacen(" id", " ,(id", 1), 2);
    let trigger = signature(input, 0, 5, Some(("(", false)));
    result(output, trigger, r#"{"signatures":[{"label":"f(x)"}]}"#);
    change(input, &SOURCE.replacen(" id", " ,(,id", 1), 3);
    let retrigger = signature(input, 0, 6, Some((",", true)));
    result(output, retrigger, "null");
}

const DEBOUNCE_SIGNATURE: &str = r#"{"activeParameter":0,"activeSignature":0,"signatures":[{"label":"f(x, y)","parameters":[{"label":"x"},{"label":"y"}]},{"label":"f(a, b)","parameters":[{"label":"a"},{"label":"b"}]}]}"#;

fn signature_in_fence(
    input: &mut impl BufRead,
    character: u32,
    kind: u32,
    trigger: Option<&str>,
    active: Option<&str>,
) -> i32 {
    let frame = expect(input, "textDocument/signatureHelp");
    for field in [
        format!(r#""uri":"{URI}""#),
        format!(r#""position":{{"line":5,"character":{character}}}"#),
        format!(r#""triggerKind":{kind}"#),
        format!(r#""isRetrigger":{}"#, active.is_some()),
    ] {
        assert!(frame.contains(&field), "{frame}");
    }
    assert_eq!(
        frame.contains(r#""triggerCharacter":"#),
        trigger.is_some(),
        "{frame}"
    );
    if let Some(trigger) = trigger {
        assert!(
            frame.contains(&format!(r#""triggerCharacter":"{trigger}""#)),
            "{frame}"
        );
    }
    assert_eq!(
        frame.contains(r#""activeSignatureHelp":"#),
        active.is_some(),
        "{frame}"
    );
    if let Some(active) = active {
        assert!(
            frame.contains(&format!(r#""activeSignatureHelp":{active}"#)),
            "{frame}"
        );
    }
    id(&frame)
}

fn signature_debounce_flow(input: &mut impl BufRead, output: &mut impl Write) {
    result(
        output,
        signature_in_fence(input, 1, 1, None, None),
        DEBOUNCE_SIGNATURE,
    );
    change(input, &SOURCE.replacen("let x", "0let x", 1), 1);
    change(input, &SOURCE.replacen("let x", "10lezt x", 1), 2);
    let refreshed =
        DEBOUNCE_SIGNATURE.replacen(r#""activeParameter":0"#, r#""activeParameter":1"#, 1);
    result(
        output,
        signature_in_fence(input, 5, 3, None, Some(DEBOUNCE_SIGNATURE)),
        &refreshed,
    );
    result(
        output,
        signature_in_fence(input, 5, 3, None, Some(&refreshed)),
        r#"{"signatures":[{"label":"stale"}]}"#,
    );
    result(
        output,
        signature_in_fence(input, 5, 3, None, Some(&refreshed)),
        "null",
    );
}

fn signature_cancel_flow(input: &mut impl BufRead, output: &mut impl Write) {
    result(
        output,
        signature_in_fence(input, 1, 1, None, None),
        DEBOUNCE_SIGNATURE,
    );
    let typed = SOURCE.replacen("let x", "le,t x", 1);
    change(input, &typed, 1);
    result(
        output,
        signature_in_fence(input, 3, 2, Some(","), Some(DEBOUNCE_SIGNATURE)),
        DEBOUNCE_SIGNATURE,
    );
    result(
        output,
        signature_in_fence(input, 4, 1, None, Some(DEBOUNCE_SIGNATURE)),
        DEBOUNCE_SIGNATURE,
    );
    for _ in 0..9 {
        result(
            output,
            signature_in_fence(input, 1, 1, None, None),
            DEBOUNCE_SIGNATURE,
        );
    }
    change(input, &typed.replacen("le,t", "le文,t", 1), 2);
    result(
        output,
        signature_in_fence(input, 1, 1, None, None),
        DEBOUNCE_SIGNATURE,
    );
}

fn main() {
    let mut args = std::env::args_os().skip(1);
    let mode = args.next().unwrap().into_string().unwrap();
    let crash_log = args.next().unwrap();
    panic::set_hook(Box::new(move |panic| {
        let _ = fs::write(&crash_log, panic.to_string());
    }));
    let mut input = io::stdin().lock();
    let mut output = io::stdout().lock();
    let initialize = expect(&mut input, "initialize");
    if mode == "invalid-initialize" {
        result(&mut output, id(&initialize), "{}");
        return;
    }
    if mode == "oversized-peer-id" {
        write_frame(
            &mut output,
            &format!(
                r#"{{"jsonrpc":"2.0","id":"{}","method":"workspace/applyEdit"}}"#,
                "x".repeat(257)
            ),
        );
        let mut tail = String::new();
        assert_eq!(input.read_line(&mut tail).unwrap(), 0);
        return;
    }
    if mode == "flow" {
        write_frame(
            &mut output,
            r#"{"jsonrpc":"2.0","id":"edit","method":"workspace/applyEdit","params":{"edit":{"changes":{"file:///explicit.md":[{"range":{"start":{"line":0,"character":0},"end":{"line":0,"character":0}},"newText":"bad"}]}}}}"#,
        );
        let reply = read_frame(&mut input);
        for field in [
            r#""id":"edit""#,
            r#""code":-32601"#,
            r#""message":"Method not implemented""#,
        ] {
            assert!(reply.contains(field), "{reply}");
        }
        write_frame(
            &mut output,
            r#"{"jsonrpc":"2.0","method":"window/showMessage","params":{"type":1,"message":"ignored"}}"#,
        );
    }
    let capabilities = match mode.as_str() {
        "no-sync" => r#"{"capabilities":{"hoverProvider":true}}"#,
        "references-no-sync" => r#"{"capabilities":{"referencesProvider":true}}"#,
        "references-unsupported" => {
            r#"{"capabilities":{"referencesProvider":false,"textDocumentSync":2}}"#
        }
        "references" | "references-code" | "references-disconnect" | "references-pointer" => {
            r#"{"capabilities":{"referencesProvider":{},"textDocumentSync":2}}"#
        }
        "signature-request" | "signature-trigger" | "signature-debounce" | "signature-cancel" => {
            r#"{"capabilities":{"signatureHelpProvider":{"triggerCharacters":["("],"retriggerCharacters":[","]},"textDocumentSync":2}}"#
        }
        _ => r#"{"capabilities":{"hoverProvider":true,"textDocumentSync":2}}"#,
    };
    result(&mut output, id(&initialize), capabilities);
    assert!(expect(&mut input, "initialized").contains(r#""params":{}"#));
    let open = expect(&mut input, "textDocument/didOpen");
    for field in [
        format!(r#""uri":"{URI}""#),
        r#""languageId":"markdown""#.into(),
        r#""version":0"#.into(),
        format!(r#""text":{SOURCE:?}"#),
    ] {
        assert!(open.contains(&field), "{open}");
    }
    match mode.as_str() {
        "flow" => flow(&mut input, &mut output),
        "references" => references_flow(&mut input, &mut output),
        "signature-request" => signature_request_flow(&mut input, &mut output),
        "signature-trigger" => signature_trigger_flow(&mut input, &mut output),
        "signature-debounce" => signature_debounce_flow(&mut input, &mut output),
        "signature-cancel" => signature_cancel_flow(&mut input, &mut output),
        "references-pointer" => {
            let request = references(&mut input, 0, 3);
            let name = location(URI, 0, 3, 5);
            let tail = location(URI, 2, 0, 4);
            result(&mut output, request, &format!("[{name},{tail}]"));
        }
        "references-code" => {
            let request = references(&mut input, 5, 0);
            let body = location(URI, 5, 0, 3);
            let header = location(URI, 4, 0, 3);
            let end = location(URI, 6, 0, 3);
            result(&mut output, request, &format!("[{header},{body},{end}]"));
        }
        "references-no-sync" => {
            let request = references(&mut input, 0, 3);
            result(
                &mut output,
                request,
                &format!("[{}]", location(URI, 0, 3, 5)),
            );
        }
        "references-unsupported" => {}
        "references-disconnect" => {
            let request = references(&mut input, 0, 3);
            result(
                &mut output,
                request,
                &format!("[{}]", location(URI, 0, 3, 5)),
            );
            references(&mut input, 0, 3);
            return;
        }
        "presentation" => {
            let point = r#"{"contents":{"kind":"plaintext","value":"hover point"}}"#;
            for (line, character, response) in [
                (0, 3, point),
                (0, 3, point),
                (5, 0, point),
                (4, 1, point),
                (5, 0, point),
                (
                    0,
                    3,
                    r#"{"contents":"hover point","range":{"start":{"line":0,"character":3},"end":{"line":2,"character":4}}}"#,
                ),
                (0, 3, point),
            ] {
                let request = hover(&mut input, line, character);
                result(&mut output, request, response);
            }
        }
        "no-sync" => {
            let request = hover(&mut input, 0, 3);
            result(&mut output, request, r#"{"contents":"opening snapshot"}"#);
        }
        "disconnect" => {
            hover(&mut input, 0, 3);
            return;
        }
        _ => panic!("unknown fixture mode: {mode}"),
    }
    assert!(expect(&mut input, "textDocument/didClose").contains(URI));
    let shutdown = expect(&mut input, "shutdown");
    result(&mut output, id(&shutdown), "null");
    expect(&mut input, "exit");
}
