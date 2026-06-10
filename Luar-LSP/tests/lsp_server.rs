use luar_lsp::json::Json;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};

struct LspClient {
    child: Child,
    reader: BufReader<std::process::ChildStdout>,
    next_id: i64,
}

impl LspClient {
    fn start() -> LspClient {
        let exe = env!("CARGO_BIN_EXE_luar-lsp");
        let mut child = Command::new(exe)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("server should start");
        let reader = BufReader::new(child.stdout.take().unwrap());
        LspClient {
            child,
            reader,
            next_id: 1,
        }
    }

    fn send(&mut self, msg: &Json) {
        let body = msg.to_string();
        let stdin = self.child.stdin.as_mut().unwrap();
        write!(stdin, "Content-Length: {}\r\n\r\n{}", body.len(), body).unwrap();
        stdin.flush().unwrap();
    }

    fn request(&mut self, method: &str, params: Json) -> Json {
        let id = self.next_id;
        self.next_id += 1;
        self.send(&Json::obj(vec![
            ("jsonrpc", Json::str("2.0")),
            ("id", Json::int(id)),
            ("method", Json::str(method)),
            ("params", params),
        ]));
        loop {
            let msg = self.read_message();
            if msg.get("id").and_then(|i| i.as_i64()) == Some(id) {
                return msg.get("result").cloned().unwrap_or(Json::Null);
            }
        }
    }

    fn notify(&mut self, method: &str, params: Json) {
        self.send(&Json::obj(vec![
            ("jsonrpc", Json::str("2.0")),
            ("method", Json::str(method)),
            ("params", params),
        ]));
    }

    fn read_message(&mut self) -> Json {
        let mut content_length = 0usize;
        loop {
            let mut header = String::new();
            self.reader.read_line(&mut header).unwrap();
            let header = header.trim();
            if header.is_empty() {
                break;
            }
            if let Some(v) = header.strip_prefix("Content-Length:") {
                content_length = v.trim().parse().unwrap();
            }
        }
        let mut buf = vec![0u8; content_length];
        self.reader.read_exact(&mut buf).unwrap();
        Json::parse(&String::from_utf8_lossy(&buf)).unwrap()
    }

    fn shutdown(&mut self) {
        let _ = self.request("shutdown", Json::Null);
        self.notify("exit", Json::Null);
        let _ = self.child.wait();
    }
}

fn setup_workspace() -> (PathBuf, String) {
    let root = std::env::temp_dir().join(format!("luar_lsp_e2e_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let main = root.join("main.luar");
    let source = "-- Doubles a number.\nlocal function double(x: number): number\n  return x * 2\nend\nlocal result = double(21)\n";
    std::fs::write(&main, source).unwrap();
    (root, source.to_string())
}

#[test]
fn lsp_end_to_end() {
    let (root, source) = setup_workspace();
    let main = root.join("main.luar");
    let main_uri = luar_lsp::lsp::path_to_uri(&main);
    let root_uri = luar_lsp::lsp::path_to_uri(&root);

    let mut client = LspClient::start();

    let init = client.request(
        "initialize",
        Json::obj(vec![
            ("rootUri", Json::str(root_uri)),
            (
                "initializationOptions",
                Json::obj(vec![
                    ("inlayHints", Json::Bool(true)),
                    ("showMutability", Json::Bool(true)),
                ]),
            ),
        ]),
    );
    assert!(init.path(&["capabilities", "hoverProvider"]).is_some());

    client.notify("initialized", Json::obj(vec![]));
    client.notify(
        "textDocument/didOpen",
        Json::obj(vec![(
            "textDocument",
            Json::obj(vec![
                ("uri", Json::str(main_uri.clone())),
                ("languageId", Json::str("luar")),
                ("version", Json::int(1)),
                ("text", Json::str(source.clone())),
            ]),
        )]),
    );

    let hover = client.request(
        "textDocument/hover",
        Json::obj(vec![
            (
                "textDocument",
                Json::obj(vec![("uri", Json::str(main_uri.clone()))]),
            ),
            (
                "position",
                Json::obj(vec![("line", Json::int(4)), ("character", Json::int(16))]),
            ),
        ]),
    );
    let hover_text = hover
        .path(&["contents", "value"])
        .and_then(|v| v.as_str())
        .unwrap_or("");
    assert!(
        hover_text.contains("function double(x: number) -> number"),
        "hover was: {hover_text}"
    );
    assert!(
        hover_text.contains("Doubles a number."),
        "hover should include the doc comment, was: {hover_text}"
    );

    let hints = client.request(
        "textDocument/inlayHint",
        Json::obj(vec![
            (
                "textDocument",
                Json::obj(vec![("uri", Json::str(main_uri.clone()))]),
            ),
            (
                "range",
                Json::obj(vec![
                    (
                        "start",
                        Json::obj(vec![("line", Json::int(0)), ("character", Json::int(0))]),
                    ),
                    (
                        "end",
                        Json::obj(vec![("line", Json::int(10)), ("character", Json::int(0))]),
                    ),
                ]),
            ),
        ]),
    );
    let hints = hints.as_array().unwrap_or(&[]).to_vec();
    let labels: Vec<String> = hints
        .iter()
        .filter_map(|h| h.get("label").and_then(|l| l.as_str()).map(String::from))
        .collect();
    assert!(
        labels.iter().any(|l| l.contains("mut number")),
        "expected a ': mut number' hint, got {labels:?}"
    );

    let updated = format!("{source}local q = result.\n");
    client.notify(
        "textDocument/didChange",
        Json::obj(vec![
            (
                "textDocument",
                Json::obj(vec![
                    ("uri", Json::str(main_uri.clone())),
                    ("version", Json::int(2)),
                ]),
            ),
            (
                "contentChanges",
                Json::Array(vec![Json::obj(vec![("text", Json::str(updated))])]),
            ),
        ]),
    );

    let completion = client.request(
        "textDocument/completion",
        Json::obj(vec![
            (
                "textDocument",
                Json::obj(vec![("uri", Json::str(main_uri.clone()))]),
            ),
            (
                "position",
                Json::obj(vec![("line", Json::int(5)), ("character", Json::int(10))]),
            ),
        ]),
    );
    let items = completion.as_array().unwrap_or(&[]).to_vec();
    let item_labels: Vec<&str> = items
        .iter()
        .filter_map(|i| i.get("label").and_then(|l| l.as_str()))
        .collect();
    assert!(
        item_labels.contains(&"double"),
        "value completion should offer the local function, got {} items",
        item_labels.len()
    );

    client.shutdown();
    let _ = std::fs::remove_dir_all(&root);
}
