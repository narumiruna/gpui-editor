//! Std-only framed peer for the live completion tests.

use std::{
    fs,
    io::{self, BufRead, Write},
    panic,
};

const URI: &str = "file:///completion-draft.md";
const SOURCE: &str = "Lead\r\n\r\n```js\r\n// 😀foo + foo\n```\r\n\r\n```js\r\nfoo\r\n```\r\n";

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

fn complete(frame: &str) -> i32 {
    method(frame, "textDocument/completion");
    has(
        frame,
        &format!(
            r#""params":{{"textDocument":{{"uri":"{URI}"}},"position":{{"line":3,"character":7}},"context":{{"triggerKind":1}}}}"#
        ),
    );
    id(frame)
}

fn range(start: (u32, u32), end: (u32, u32)) -> String {
    let pos = |(line, character)| format!(r#"{{"line":{line},"character":{character}}}"#);
    format!(r#"{{"start":{},"end":{}}}"#, pos(start), pos(end))
}

fn edit(start: (u32, u32), end: (u32, u32), text: &str) -> String {
    format!(r#"{{"range":{},"newText":{text:?}}}"#, range(start, end))
}

fn choice(primary: &str, additional: &str) -> String {
    format!(r#"[{{"label":"choice","textEdit":{primary},"additionalTextEdits":[{additional}]}}]"#)
}

fn did_change(input: &mut impl BufRead, version: u32, expected: &str) {
    let frame = read(input);
    method(&frame, "textDocument/didChange");
    has(&frame, &format!(r#""version":{version}"#));
    has(
        &frame,
        &format!(r#""contentChanges":[{{"text":{expected:?}}}]"#),
    );
}

fn flow(input: &mut impl BufRead, output: &mut impl Write) {
    let synced = SOURCE.replacen("Lead", "Changed", 1);
    did_change(input, 1, &synced);
    let request = complete(&read(input));
    send(
        output,
        r#"{"jsonrpc":"2.0","id":"peer-edit","method":"workspace/applyEdit","params":{}}"#,
    );
    let reply = read(input);
    has(&reply, r#""id":"peer-edit""#);
    has(&reply, r#""code":-32601"#);
    result(
        output,
        request,
        &format!(
            r#"{{"isIncomplete":true,"itemDefaults":{{"editRange":{{"insert":{},"replace":{}}},"insertTextFormat":1,"commitCharacters":["."]}},"items":[{{"label":"first","textEditText":"alpha"}},{{"label":"[literal]","textEdit":{},"additionalTextEdits":[{}],"documentation":{{"kind":"markdown","value":"[not opened](https://example.invalid/)"}},"command":{{"title":"ignored","command":"fixture.must-not-run"}},"data":{{"uri":"file:///never-opened"}}}},{{"label":"snippet","insertText":"f($1)","insertTextFormat":2}}]}}"#,
            range((3, 5), (3, 7)),
            range((3, 5), (3, 8)),
            edit((3, 6), (3, 8), "élan"),
            edit((3, 11), (3, 14), "tail")
        ),
    );
    let applied = synced.replacen("foo + foo", "félan + tail", 1);
    did_change(input, 2, &applied);
    did_change(input, 3, &synced);
    did_change(input, 4, &applied);
}

fn rejected(input: &mut impl BufRead, output: &mut impl Write) {
    let valid = edit((3, 5), (3, 8), "bar");
    let too_large = "x".repeat(16 * 1024 + 1);
    let big = format!(r#"{{"label":"big","insertText":{too_large:?}}}"#);
    let many = vec![r#"{"label":"many"}"#; 33].join(",");
    let several = vec![edit((3, 14), (3, 14), ""); 17].join(",");
    for value in [
        "null".into(),
        "{}".into(),
        "[]".into(),
        choice(&edit((3, 4), (3, 8), "bar"), ""), // Split surrogate.
        choice(&edit((3, 5), (3, 15), "bar"), ""), // Beyond line.
        choice(&edit((3, 8), (3, 7), "bar"), ""), // Reversed.
        choice(&edit((3, 5), (4, 0), "bar"), ""), // Across separator.
        choice(&edit((3, 5), (3, 8), "foo"), ""), // No change.
        choice(&valid, &edit((0, 0), (0, 0), "")), // Outside-body no-op.
        choice(&valid, &edit((3, 4), (3, 5), "bad")), // Invalid side edit.
        choice(&valid, &edit((7, 0), (7, 3), "bar")), // Other fence.
        choice(&valid, &edit((3, 6), (3, 8), "bar")), // Overlap.
        format!("[{big},{big}]"),
        format!("[{many}]"),
        choice(&valid, &several),
        format!(
            r#"{{"isIncomplete":false,"itemDefaults":{{"insertTextFormat":2}},"items":[{{"label":"f($1)"}}]}}"#
        ),
    ] {
        result(output, complete(&read(input)), &value);
    }
    for value in [
        choice(&valid, &edit((3, 14), (3, 14), "\r")), // Merges with protected LF.
        choice(&edit((3, 5), (3, 14), "``"), &edit((3, 0), (3, 5), "`")), // Early close.
    ] {
        result(output, complete(&read(input)), &value);
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
        "unsupported" => r#"{"capabilities":{"textDocumentSync":1,"completionProvider":false}}"#,
        "no-sync" => r#"{"capabilities":{"completionProvider":{}}}"#,
        _ => {
            r#"{"capabilities":{"textDocumentSync":1,"completionProvider":{"triggerCharacters":["."]}}}"#
        }
    };
    result(&mut output, id(&initialize), capabilities);
    method(&read(&mut input), "initialized");
    let open = read(&mut input);
    method(&open, "textDocument/didOpen");
    has(&open, URI);
    has(&open, &format!(r#""text":{SOURCE:?}"#));
    has(&open, r#""version":0"#);
    let simple = r#"[{"label":"server","insertText":"bar"}]"#;
    let close = match mode.as_str() {
        "flow" | "rejected" | "unsupported" | "no-sync" | "stale" => {
            match mode.as_str() {
                "flow" => flow(&mut input, &mut output),
                "rejected" => rejected(&mut input, &mut output),
                "unsupported" => {
                    did_change(&mut input, 1, &SOURCE.replacen("Lead", "Changed", 1));
                }
                "stale" => {
                    let stale = complete(&read(&mut input));
                    did_change(&mut input, 1, &SOURCE.replacen("Lead", "Changed", 1));
                    result(&mut output, stale, simple);
                    result(
                        &mut output,
                        complete(&read(&mut input)),
                        r#"[{"label":"fresh","insertText":"baz"}]"#,
                    );
                }
                _ => {}
            }
            read(&mut input)
        }
        "repeat" => loop {
            let frame = read(&mut input);
            if frame.contains(r#""method":"textDocument/didClose""#) {
                break frame;
            }
            result(&mut output, complete(&frame), simple);
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
