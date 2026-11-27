//! Std-only diagnostic peer for the live editor tests.

use std::{
    io::{self, BufRead, Write},
    panic,
};

const URI: &str = "file:///explicit.md";
const SOURCE: &str =
    "Outside 😀\r\n\r\n```json\r\n[\"😀x\",]\r\n```\r\n\r\n```rust\r\nsecond\r\n```\r\n";

fn read(input: &mut impl BufRead, method: &str) -> String {
    let mut header = String::new();
    input.read_line(&mut header).unwrap();
    let len: usize = header
        .strip_prefix("Content-Length: ")
        .expect(&header)
        .trim_end()
        .parse()
        .unwrap();
    header.clear();
    input.read_line(&mut header).unwrap();
    assert_eq!(header, "\r\n");
    let mut bytes = vec![0; len];
    input.read_exact(&mut bytes).unwrap();
    let frame = String::from_utf8(bytes).unwrap();
    assert!(
        frame.contains(&format!(r#""method":"{method}""#)),
        "{frame}"
    );
    frame
}

fn send(output: &mut impl Write, body: &str) {
    write!(output, "Content-Length: {}\r\n\r\n{body}", body.len()).unwrap();
    output.flush().unwrap();
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

fn ping(input: &mut impl BufRead) -> i32 {
    id(&read(input, "textDocument/hover"))
}

fn pong(output: &mut impl Write, id: i32, label: &str) {
    send(
        output,
        &format!(r#"{{"jsonrpc":"2.0","id":{id},"result":{{"contents":"{label}"}}}}"#),
    );
}

fn publish(output: &mut impl Write, uri: &str, version: &str, diagnostics: &str) {
    send(
        output,
        &format!(
            r#"{{"jsonrpc":"2.0","method":"textDocument/publishDiagnostics","params":{{"uri":"{uri}",{version}"diagnostics":{diagnostics}}}}}"#
        ),
    );
}

fn item(line: u32, start: u32, end: u32, text: &str) -> String {
    format!(
        r#"{{"range":{{"start":{{"line":{line},"character":{start}}},"end":{{"line":{line},"character":{end}}}}},"severity":2,"message":"{text}","codeDescription":{{"href":"file:///do-not-open"}},"relatedInformation":[],"data":{{"command":"do not run"}}}}"#
    )
}

fn one(output: &mut impl Write, version: &str, label: &str) {
    publish(output, URI, version, &format!("[{}]", item(3, 2, 4, label)));
}

fn flow(input: &mut impl BufRead, output: &mut impl Write) {
    let zero = r#""version":0,"#;
    let current = r#""version":1,"#;
    let request = ping(input);
    one(output, zero, "disabled");
    pong(output, request, "disabled");

    let request = ping(input);
    one(output, zero, "earlier same version");
    publish(
        output,
        URI,
        zero,
        &format!(
            "[{},{},{},{},{},{}]",
            item(0, 0, 1, "prose"),
            item(2, 0, 1, "opening fence"),
            item(3, 2, 4, r#"[link](file:///do-not-open) <b>😀</b>\u202e"#),
            item(3, 5, 5, "point"),
            item(4, 0, 1, "closing fence"),
            item(7, 0, 1, "different fence"),
        ),
    );
    pong(output, request, "accepted");

    let request = ping(input);
    for (uri, version) in [
        ("file:///different.md", zero),
        (URI, r#""version":1,"#),
        (URI, r#""version":"0","#),
        (URI, r#""version":null,"#),
        (URI, ""),
    ] {
        publish(output, uri, version, "[]");
    }
    let valid = item(3, 2, 4, "must not publish part of a batch");
    for invalid in [
        item(3, 3, 4, "split surrogate"),
        item(3, 4, 2, "reversed"),
        item(3, 8, 9, "past the CRLF line end"),
        item(100, 0, 1, "past last line"),
        "{}".into(),
        item(3, 2, 4, &"a".repeat(2049)),
    ] {
        publish(output, URI, zero, &format!("[{valid},{invalid}]"));
    }
    publish(
        output,
        URI,
        zero,
        &format!("[{}]", [valid.as_str(); 129].join(",")),
    );
    let long = item(3, 2, 4, &"b".repeat(2048));
    publish(
        output,
        URI,
        zero,
        &format!("[{}]", [long.as_str(); 9].join(",")),
    );
    pong(output, request, "invalid");

    let request = ping(input);
    publish(
        output,
        URI,
        zero,
        &format!(
            "[{},{}]",
            item(3, 2, 4, "previous fence"),
            item(7, 0, 6, "active fence")
        ),
    );
    pong(output, request, "second fence");

    let request = ping(input);
    one(output, zero, "during composition");
    pong(output, request, "composing");

    let request = ping(input);
    one(output, zero, "before edit");
    pong(output, request, "before edit");
    let old_request = ping(input);
    one(output, zero, "late opening revision");
    pong(output, old_request, "old reply");
    let change = read(input, "textDocument/didChange");
    assert!(change.contains(r#""version":1"#), "{change}");
    assert!(
        change.contains(&format!(r#""text":{:?}"#, SOURCE.replacen('x', "élan", 1))),
        "{change}"
    );
    let request = ping(input);
    publish(output, URI, zero, "[]");
    publish(
        output,
        URI,
        current,
        &format!("[{}]", item(3, 4, 8, "next revision")),
    );
    one(output, zero, "stale cannot overwrite");
    pong(output, request, "updated");

    let request = ping(input);
    publish(output, URI, current, "[]");
    one(output, zero, "stale cannot resurrect");
    pong(output, request, "clear");

    let request = ping(input);
    one(output, current, "before disable");
    pong(output, request, "disable");

    let request = ping(input);
    one(output, current, "before detach");
    pong(output, request, "detach");
}

fn no_sync(input: &mut impl BufRead, output: &mut impl Write) {
    let request = ping(input);
    one(output, r#""version":0,"#, "opening revision");
    pong(output, request, "accepted");
    // A shutdown handshake would indicate that the attachment did not detach on EOF.
    let request = ping(input);
    one(output, r#""version":0,"#, "until EOF");
    pong(output, request, "exit");
}

fn main() {
    let mut args = std::env::args().skip(1);
    let mode = args.next().unwrap();
    let error_path = args.next().unwrap();
    panic::set_hook(Box::new(move |info| {
        let _ = std::fs::write(&error_path, info.to_string());
    }));
    let mut input = io::stdin().lock();
    let mut output = io::stdout().lock();
    let init = read(&mut input, "initialize");
    assert!(init.contains(r#""publishDiagnostics":{"versionSupport":true}"#));
    let sync = i32::from(mode == "flow");
    send(
        &mut output,
        &format!(
            r#"{{"jsonrpc":"2.0","id":{},"result":{{"capabilities":{{"textDocumentSync":{sync},"hoverProvider":true}}}}}}"#,
            id(&init)
        ),
    );
    read(&mut input, "initialized");
    let open = read(&mut input, "textDocument/didOpen");
    assert!(open.contains(&format!(r#""uri":"{URI}""#)));
    assert!(open.contains(&format!(r#""text":{SOURCE:?}"#)), "{open}");
    assert!(open.contains(r#""version":0"#), "{open}");
    match mode.as_str() {
        "flow" => flow(&mut input, &mut output),
        "no-sync" => {
            no_sync(&mut input, &mut output);
            return;
        }
        _ => panic!("unexpected mode"),
    }
    read(&mut input, "textDocument/didClose");
    let shutdown = read(&mut input, "shutdown");
    send(
        &mut output,
        &format!(
            r#"{{"jsonrpc":"2.0","id":{},"result":null}}"#,
            id(&shutdown)
        ),
    );
    read(&mut input, "exit");
}
