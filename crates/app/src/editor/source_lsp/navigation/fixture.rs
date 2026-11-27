//! Std-only framed peer for the live navigation tests.

use std::{
    fs,
    io::{self, BufRead, Write},
    panic,
};

const URI: &str = "file:///navigation-fixture.md";
const SOURCE: &str = "Lead\r\n\r\n```rs\r\nfn demo() {\r\n    // 😀 target one\r\n    first();\r\n}\r\n```\r\n\r\n```rs\r\nother();\r\n```\r\n";

fn read(input: &mut impl BufRead) -> String {
    let mut header = String::new();
    input.read_line(&mut header).unwrap();
    let size = header
        .strip_prefix("Content-Length: ")
        .unwrap_or_else(|| panic!("missing Content-Length: {header:?}"))
        .trim_end()
        .parse()
        .unwrap();
    header.clear();
    input.read_line(&mut header).unwrap();
    assert_eq!(header, "\r\n");
    let mut bytes = vec![0; size];
    input.read_exact(&mut bytes).unwrap();
    String::from_utf8(bytes).unwrap()
}

fn has(frame: &str, part: &str) {
    assert!(frame.contains(part), "expected {part} in {frame}");
}

fn method(frame: &str, name: &str) {
    has(frame, &format!(r#""method":"{name}""#));
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

fn send(output: &mut impl Write, body: &str) {
    write!(output, "Content-Length: {}\r\n\r\n{body}", body.len()).unwrap();
    output.flush().unwrap();
}

fn result(output: &mut impl Write, id: i32, value: &str) {
    send(
        output,
        &format!(r#"{{"jsonrpc":"2.0","id":{id},"result":{value}}}"#),
    );
}

fn request(frame: &str, kind: &str) -> i32 {
    method(frame, &format!("textDocument/{kind}"));
    has(
        frame,
        &format!(
            r#""params":{{"textDocument":{{"uri":"{URI}"}},"position":{{"line":4,"character":10}}}}"#
        ),
    );
    id(frame)
}

fn location(uri: &str, start: (u32, u32), end: (u32, u32)) -> String {
    let pos = |(line, character)| format!(r#"{{"line":{line},"character":{character}}}"#);
    format!(
        r#"{{"uri":"{uri}","range":{{"start":{},"end":{}}}}}"#,
        pos(start),
        pos(end)
    )
}

fn target() -> String {
    location(URI, (5, 4), (5, 9))
}

fn flow(input: &mut impl BufRead, output: &mut impl Write) {
    let changed = read(input);
    method(&changed, "textDocument/didChange");
    let synced = SOURCE.replacen("Lead", "New lead", 1);
    has(
        &changed,
        &format!(r#""contentChanges":[{{"text":{synced:?}}}]"#),
    );
    has(
        &changed,
        &format!(r#""textDocument":{{"uri":"{URI}","version":1}}"#),
    );
    let first = target();
    let second = location(URI, (4, 10), (4, 16));
    for (kind, value) in [
        ("definition", format!("[{first},{second}]")),
        ("declaration", first.clone()),
        ("typeDefinition", format!("[{first},{second}]")),
        ("implementation", first.clone()),
    ] {
        let request = request(&read(input), kind);
        if kind == "definition" {
            result(output, request + 500, &second);
            send(
                output,
                r#"{"jsonrpc":"2.0","id":"peer-edit","method":"workspace/applyEdit","params":{"edit":{"changes":{}}}}"#,
            );
            let reply = read(input);
            has(&reply, r#""id":"peer-edit""#);
            has(&reply, r#""code":-32601"#);
        }
        result(output, request, &value);
    }
}

fn bad_results(input: &mut impl BufRead, output: &mut impl Write) {
    let valid = target();
    let external = location("file:///not-loaded.md", (5, 4), (5, 9));
    for value in [
        "null".into(),
        "[]".into(),
        format!("[{external},{valid}]"),
        format!("[{valid},{external}]"),
        format!("[{valid},{{}}]"),
        r#"[{"targetUri":"file:///navigation-fixture.md","targetRange":{"start":{"line":5,"character":4},"end":{"line":5,"character":9}},"targetSelectionRange":{"start":{"line":5,"character":4},"end":{"line":5,"character":9}}}]"#.into(),
        location(URI, (4, 8), (4, 9)), // Split surrogate pair.
        location(URI, (4, 7), (4, 8)), // Invalid end also rejects the jump.
        location(URI, (5, 13), (5, 13)), // Past line end, not a position in CRLF.
        location(URI, (5, 9), (5, 4)), // Reversed.
        location(URI, (2, 0), (2, 3)), // Opening fence.
        location(URI, (6, 0), (7, 0)), // Crosses the protected line break.
        location(URI, (0, 0), (0, 4)), // Prose.
        location(URI, (10, 0), (10, 5)), // Other fence.
        format!("[{}]", vec![valid.as_str(); 65].join(",")),
    ] {
        result(output, request(&read(input), "definition"), &value);
    }
}

fn main() {
    let mut args = std::env::args_os().skip(1);
    let mode = args.next().unwrap().into_string().unwrap();
    let log = args.next().unwrap();
    panic::set_hook(Box::new(move |panic| {
        let _ = fs::write(&log, panic.to_string());
    }));
    let mut input = io::stdin().lock();
    let mut output = io::stdout().lock();
    let initialize = read(&mut input);
    method(&initialize, "initialize");
    let capabilities = match mode.as_str() {
        "flow" => {
            r#"{"capabilities":{"textDocumentSync":1,"definitionProvider":true,"declarationProvider":{},"typeDefinitionProvider":true,"implementationProvider":true}}"#
        }
        "unsupported" => r#"{"capabilities":{"textDocumentSync":1,"definitionProvider":false}}"#,
        "no-sync" => r#"{"capabilities":{"definitionProvider":true}}"#,
        _ => r#"{"capabilities":{"textDocumentSync":1,"definitionProvider":true}}"#,
    };
    result(&mut output, id(&initialize), capabilities);
    method(&read(&mut input), "initialized");
    let open = read(&mut input);
    method(&open, "textDocument/didOpen");
    has(&open, &format!(r#""text":{SOURCE:?}"#));
    has(&open, &format!(r#""uri":"{URI}""#));
    has(&open, r#""version":0"#);
    let close = match mode.as_str() {
        "flow" | "bad-results" => {
            match mode.as_str() {
                "flow" => flow(&mut input, &mut output),
                _ => bad_results(&mut input, &mut output),
            }
            read(&mut input)
        }
        "unsupported" => read(&mut input),
        "no-sync" => {
            result(
                &mut output,
                request(&read(&mut input), "definition"),
                &target(),
            );
            read(&mut input)
        }
        "repeat" => loop {
            let frame = read(&mut input);
            if frame.contains(r#""method":"textDocument/didClose""#) {
                break frame;
            }
            if !frame.contains(r#""method":"textDocument/didChange""#) {
                result(&mut output, request(&frame, "definition"), &target());
            }
        },
        _ => panic!("unknown fixture mode {mode}"),
    };
    method(&close, "textDocument/didClose");
    has(&close, URI);
    let shutdown = read(&mut input);
    method(&shutdown, "shutdown");
    result(&mut output, id(&shutdown), "null");
    method(&read(&mut input), "exit");
}
