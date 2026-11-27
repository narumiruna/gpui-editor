//! A std-only language server fixture. The integration test compiles and runs it.

use std::{
    io::{self, BufRead, Write},
    thread,
    time::Duration,
};

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
    assert_eq!(header, "\r\n");
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

fn write_frame(output: &mut impl Write, body: &str) {
    // Flush half a header first; fragment the wire without relying on timing.
    output.write_all(b"Content-Len").unwrap();
    output.flush().unwrap();
    write!(output, "gth: {}\r\n\r\n{body}", body.len()).unwrap();
    output.flush().unwrap();
}

fn result(output: &mut impl Write, id: i32, value: &str) {
    write_frame(
        output,
        &format!(r#"{{"jsonrpc":"2.0","id":{id},"result":{value}}}"#),
    );
}

fn managed(input: &mut impl BufRead, output: &mut impl Write) {
    for i in 0..12 {
        let open = expect(input, "textDocument/didOpen");
        assert!(
            open.contains(&format!(r#""uri":"file:///managed/{i}.rs""#)),
            "{open}"
        );
        assert!(
            open.contains(r#""text":"a😀b""#) && open.contains(r#""version":0"#),
            "{open}"
        );
        if i == 0 {
            write_frame(output, r#"{"jsonrpc":"2.0","method":"test/between"}"#);
        }
    }
    for i in 0..12 {
        let change = expect(input, "textDocument/didChange");
        assert!(
            change.contains(&format!(r#""uri":"file:///managed/{i}.rs""#)),
            "{change}"
        );
        assert!(
            change.contains(&format!(r#""contentChanges":[{{"text":"a😀β{i}"}}]"#)),
            "{change}"
        );
        assert!(change.contains(r#""version":1"#), "{change}");
    }
    let close = expect(input, "textDocument/didClose");
    assert!(
        close.contains(r#""uri":"file:///managed/11.rs""#),
        "{close}"
    );
    let change = expect(input, "textDocument/didChange");
    assert!(
        change.contains(r#""uri":"file:///managed/0.rs""#) && change.contains(r#""version":2"#),
        "{change}"
    );
    assert!(
        change.contains(r#""contentChanges":[{"text":"a😀β0!"}]"#),
        "{change}"
    );
    let hover = expect(input, "textDocument/hover");
    assert!(
        hover.contains(r#""uri":"file:///managed/0.rs""#) && hover.contains(r#""character":6"#),
        "{hover}"
    );
    result(
        output,
        id(&hover),
        r#"{"contents":{"kind":"plaintext","value":"fixture"}}"#,
    );
    for i in 0..11 {
        let close = expect(input, "textDocument/didClose");
        assert!(
            close.contains(&format!(r#""uri":"file:///managed/{i}.rs""#)),
            "{close}"
        );
    }
    let open = expect(input, "textDocument/didOpen");
    assert!(
        open.contains(r#""uri":"file:///shutdown.rs""#) && open.contains(r#""text":"a""#),
        "{open}"
    );
    let change = expect(input, "textDocument/didChange");
    assert!(
        change.contains(r#""uri":"file:///shutdown.rs""#) && change.contains(r#""text":"ab""#),
        "{change}"
    );
}

fn main() {
    let mut args = std::env::args().skip(1);
    let mode = args.next().unwrap();
    assert_eq!(args.next().as_deref(), Some("空 白; $(literal)"));
    assert_eq!(args.next(), None);
    assert!(std::env::vars_os().next().is_none());
    let mut input = io::stdin().lock();
    let mut output = io::stdout().lock();

    let initialize = expect(&mut input, "initialize");
    if mode == "invalid-initialize" {
        result(&mut output, id(&initialize), "{}");
        return;
    }
    if mode == "reject-initialize" {
        write_frame(
            &mut output,
            &format!(
                r#"{{"jsonrpc":"2.0","id":{},"error":{{"code":-32002,"message":"Rejected"}}}}"#,
                id(&initialize)
            ),
        );
        return;
    }
    if mode == "managed" {
        for field in [
            r#""processId":null"#,
            r#""rootUri":null"#,
            r#""snippetSupport":false"#,
        ] {
            assert!(initialize.contains(field), "{initialize}");
        }
        write_frame(
            &mut output,
            r#"{"jsonrpc":"2.0","id":"init","method":"workspace/configuration"}"#,
        );
        let reply = read_frame(&mut input);
        assert!(
            reply.contains(r#""id":"init""#) && reply.contains(r#""result":[]"#),
            "{reply}"
        );
    }
    let capabilities = match mode.as_str() {
        "editor" | "managed" => r#"{"capabilities":{"hoverProvider":true,"textDocumentSync":2}}"#,
        "unsupported-encoding" => r#"{"capabilities":{"positionEncoding":"utf-8"}}"#,
        _ => r#"{"capabilities":{"hoverProvider":true}}"#,
    };
    result(&mut output, id(&initialize), capabilities);
    let initialized = expect(&mut input, "initialized");
    assert!(initialized.contains(r#""params":{}"#));

    match mode.as_str() {
        "managed" => managed(&mut input, &mut output),
        "managed-eof" => {
            expect(&mut input, "textDocument/didOpen");
            expect(&mut input, "textDocument/hover");
            return;
        }
        "unsupported-encoding" => {
            panic!("client must not send initialized after refusing capabilities")
        }
        "editor" => {
            let open = expect(&mut input, "textDocument/didOpen");
            for field in [
                r#""text":"let id""#,
                r#""uri":"file:///editor.rs""#,
                r#""version":0"#,
            ] {
                assert!(open.contains(field), "{open}");
            }
            let change = expect(&mut input, "textDocument/didChange");
            for field in [
                r#""contentChanges":[{"text":"let name"}]"#,
                r#""uri":"file:///editor.rs""#,
                r#""version":1"#,
            ] {
                assert!(change.contains(field), "{change}");
            }
            let hover = expect(&mut input, "textDocument/hover");
            assert!(
                hover.contains(r#""position":{"character":5,"line":0}"#)
                    || hover.contains(r#""position":{"line":0,"character":5}"#),
                "{hover}"
            );
            result(
                &mut output,
                id(&hover),
                r#"{"contents":{"kind":"plaintext","value":"fixture"}}"#,
            );
            let close = expect(&mut input, "textDocument/didClose");
            assert!(close.contains(r#""textDocument":{"uri":"file:///editor.rs"}"#));
        }
        "exchange" => {
            io::stderr().write_all(&[b'x'; 256 * 1024]).unwrap();
            write_frame(&mut output, "{");
            let first = expect(&mut input, "test/hold");
            let second = expect(&mut input, "test/quick");
            result(&mut output, id(&second), r#""é😀雪""#);
            write_frame(
                &mut output,
                r#"{"jsonrpc":"2.0","id":"fixture","method":"workspace/configuration"}"#,
            );
            let reply = read_frame(&mut input);
            assert!(reply.contains(r#""id":"fixture""#) && reply.contains(r#""result":[3]"#));
            let cancel = expect(&mut input, "$/cancelRequest");
            assert_eq!(id(&cancel), id(&first));
            write_frame(
                &mut output,
                &format!(
                    r#"{{"jsonrpc":"2.0","id":{},"error":{{"code":-32800,"message":"Cancelled"}}}}"#,
                    id(&first)
                ),
            );
            let stale = expect(&mut input, "test/timeout");
            write_frame(&mut output, r#"{"jsonrpc":"2.0","method":"test/received"}"#);
            let after = expect(&mut input, "test/after-timeout");
            result(&mut output, id(&stale), "null");
            result(&mut output, id(&after), "true");
        }
        "eof" | "truncated" | "oversized" => {
            expect(&mut input, "test/disconnect");
            write_frame(&mut output, r#"{"jsonrpc":"2.0","method":"test/last"}"#);
            match mode.as_str() {
                "truncated" => output.write_all(b"Content-Length: 9\r\n\r\n{").unwrap(),
                "oversized" => output
                    .write_all(b"Content-Length: 9000000\r\n\r\n")
                    .unwrap(),
                _ => {}
            }
            output.flush().unwrap();
            return;
        }
        "unread" => {
            write_frame(
                &mut output,
                r#"{"jsonrpc":"2.0","method":"test/not-reading"}"#,
            );
            thread::sleep(Duration::from_secs(30));
            return;
        }
        "shutdown-timeout" | "exit-timeout" => {}
        _ => panic!("unknown fixture mode"),
    }

    let shutdown = expect(&mut input, "shutdown");
    if mode == "shutdown-timeout" {
        thread::sleep(Duration::from_secs(30));
        return;
    }
    result(&mut output, id(&shutdown), "null");
    expect(&mut input, "exit");
    if mode == "exit-timeout" {
        thread::sleep(Duration::from_secs(30));
    } else {
        let mut extra = String::new();
        assert_eq!(input.read_line(&mut extra).unwrap(), 0);
    }
}
