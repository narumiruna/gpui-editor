//! Std-only peer for the opt-in editor formatting tests.

use std::{
    fs,
    io::{self, BufRead, Write},
    panic,
};

const URI: &str = "file:///format-fixture.md";
const SOURCE: &str =
    "😀 id\r\n\r\nTail. [site](https://example.invalid)\r\n\r\n```rs\r\nlet x = 1;\r\n```\r\n";

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

fn formatting(frame: &str, tab: u32, spaces: bool) -> i32 {
    method(frame, "textDocument/formatting");
    has(
        frame,
        &format!(
            r#""params":{{"textDocument":{{"uri":"{URI}"}},"options":{{"tabSize":{tab},"insertSpaces":{spaces}}}}}"#
        ),
    );
    id(frame)
}

fn changed(frame: &str, version: u32, text: &str) {
    method(frame, "textDocument/didChange");
    has(frame, &format!(r#""version":{version}}}"#));
    has(frame, &format!(r#""uri":"{URI}""#));
    has(frame, &format!(r#""contentChanges":[{{"text":{text:?}}}]"#));
}

fn edit(line: u32, start: u32, end: u32, text: &str) -> String {
    format!(
        r#"{{"range":{{"start":{{"line":{line},"character":{start}}},"end":{{"line":{line},"character":{end}}}}},"newText":{text:?}}}"#
    )
}

fn prose() -> String {
    format!("[{}]", edit(0, 3, 5, "name"))
}

fn flow(input: &mut impl BufRead, output: &mut impl Write) {
    let synced = SOURCE.replacen("id", "name", 1);
    changed(&read(input), 1, &synced);
    let request = formatting(&read(input), 8, false);
    result(output, request + 500, &prose());
    send(
        output,
        r#"{"jsonrpc":"2.0","id":"peer-edit","method":"workspace/applyEdit","params":{"edit":{"changes":{}}}}"#,
    );
    let reply = read(input);
    has(&reply, r#""id":"peer-edit""#);
    has(&reply, r#""code":-32601"#);
    // The response is deliberately out of document order.
    result(
        output,
        request,
        &format!(
            "[{},{}]",
            edit(2, 0, 4, "Lead\r\nNext"),
            edit(0, 3, 7, "élan")
        ),
    );
    let formatted = synced
        .replacen("name", "élan", 1)
        .replacen("Tail", "Lead\r\nNext", 1);
    changed(&read(input), 2, &formatted);
    changed(&read(input), 3, &synced);
}

fn bad_results(input: &mut impl BufRead, output: &mut impl Write) {
    let valid = edit(0, 3, 5, "name");
    for value in [
        "null".into(),
        "[]".into(),
        r#"{"changes":{"file:///other.md":[]},"command":"not-a-format-result"}"#.into(),
        format!("[{valid},{}]", edit(0, 1, 2, "split surrogate")),
        format!("[{valid},{}]", edit(0, 6, 6, "past end of CRLF line")),
        format!("[{valid},{}]", edit(0, 4, 5, "overlap")),
        format!("[{valid},{}]", edit(2, 13, 20, "link destination")),
        format!("[{valid},{}]", edit(5, 0, 3, "read-only code")),
        format!("[{valid},{}]", edit(4, 0, 1, "fence")),
        format!("[{}]", edit(0, 3, 5, "<script>not markup</script>")),
        format!("[{}]", edit(3, 0, 0, "```text\r\n")),
    ] {
        result(output, formatting(&read(input), 2, true), &value);
    }
}

fn code(input: &mut impl BufRead, output: &mut impl Write) {
    let body = edit(5, 0, 3, "\tlet");
    for value in [
        format!("[{body},{}]", edit(4, 3, 5, "txt")),
        format!("[{body},{}]", edit(6, 0, 3, "~~~")),
        format!("[{body},{}]", edit(2, 0, 4, "Other")),
        format!("[{}]", edit(5, 0, 3, "```\r\nOutside")),
        format!("[{},{}]", edit(5, 10, 10, "\r\nlet y = \"😀\";"), body),
    ] {
        result(output, formatting(&read(input), 4, true), &value);
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
        "flow" => r#"{"capabilities":{"documentFormattingProvider":{},"textDocumentSync":2}}"#,
        "unsupported" => {
            r#"{"capabilities":{"documentFormattingProvider":false,"textDocumentSync":2}}"#
        }
        "missing" => r#"{"capabilities":{"textDocumentSync":2}}"#,
        "no-sync" => r#"{"capabilities":{"documentFormattingProvider":true}}"#,
        _ => r#"{"capabilities":{"documentFormattingProvider":true,"textDocumentSync":2}}"#,
    };
    result(&mut output, id(&initialize), capabilities);
    method(&read(&mut input), "initialized");
    let open = read(&mut input);
    method(&open, "textDocument/didOpen");
    for part in [
        format!(r#""text":{SOURCE:?}"#),
        format!(r#""uri":"{URI}""#),
        r#""version":0"#.into(),
    ] {
        has(&open, &part);
    }
    let close = match mode.as_str() {
        "flow" => {
            flow(&mut input, &mut output);
            read(&mut input)
        }
        "bad-results" => {
            bad_results(&mut input, &mut output);
            read(&mut input)
        }
        "code" => {
            code(&mut input, &mut output);
            read(&mut input)
        }
        "unsupported" | "missing" => read(&mut input),
        "no-sync" => {
            result(
                &mut output,
                formatting(&read(&mut input), 2, true),
                &prose(),
            );
            read(&mut input)
        }
        "repeat" => loop {
            let frame = read(&mut input);
            if frame.contains(r#""method":"textDocument/didClose""#) {
                break frame;
            }
            if !frame.contains(r#""method":"textDocument/didChange""#) {
                result(&mut output, formatting(&frame, 2, true), &prose());
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
