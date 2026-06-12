use crate::completion::{self, FileView};
use crate::infer::BindingKind;
use crate::json::Json;
use crate::project::Project;
use crate::types::Type;
use luar::ast::Mutability;
use std::collections::BTreeMap;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy)]
pub struct Settings {
    pub inlay_hints: bool,
    pub show_mutability: bool,
    pub auto_import: bool,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings {
            inlay_hints: true,
            show_mutability: true,
            auto_import: true,
        }
    }
}

pub struct Server {
    project: Option<Project>,
    settings: Settings,
    open_docs: std::collections::HashSet<String>,
}

impl Default for Server {
    fn default() -> Server {
        Server {
            project: None,
            settings: Settings::default(),
            open_docs: std::collections::HashSet::new(),
        }
    }
}

pub fn uri_to_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    let rest = rest.strip_prefix('/').unwrap_or(rest);
    let mut decoded = String::new();
    let bytes = rest.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() + 1 && i + 2 < bytes.len() {
            if let Ok(v) = u8::from_str_radix(&rest[i + 1..i + 3], 16) {
                decoded.push(v as char);
                i += 3;
                continue;
            }
        }
        decoded.push(bytes[i] as char);
        i += 1;
    }
    let decoded = decoded.replace('/', "\\");
    Some(crate::project::normalize(Path::new(&decoded)))
}

pub fn path_to_uri(path: &Path) -> String {
    let s = path.to_string_lossy().replace('\\', "/");
    let mut out = String::from("file:///");
    for c in s.chars() {
        match c {
            ':' => out.push_str("%3A"),
            ' ' => out.push_str("%20"),
            c => out.push(c),
        }
    }
    out
}

fn read_message(stdin: &mut impl BufRead) -> Option<Json> {
    let mut content_length: usize = 0;
    loop {
        let mut header = String::new();
        if stdin.read_line(&mut header).ok()? == 0 {
            return None;
        }
        let header = header.trim();
        if header.is_empty() {
            break;
        }
        if let Some(v) = header.strip_prefix("Content-Length:") {
            content_length = v.trim().parse().ok()?;
        }
    }
    let mut buf = vec![0u8; content_length];
    stdin.read_exact(&mut buf).ok()?;
    Json::parse(&String::from_utf8_lossy(&buf)).ok()
}

fn write_message(stdout: &mut impl Write, msg: &Json) {
    let body = msg.to_string();
    let _ = write!(stdout, "Content-Length: {}\r\n\r\n{}", body.len(), body);
    let _ = stdout.flush();
}

fn response(id: &Json, result: Json) -> Json {
    Json::obj(vec![
        ("jsonrpc", Json::str("2.0")),
        ("id", id.clone()),
        ("result", result),
    ])
}

impl Server {
    pub fn run(&mut self) {
        let stdin = std::io::stdin();
        let mut reader = stdin.lock();
        let stdout = std::io::stdout();
        let mut writer = stdout.lock();
        loop {
            let Some(msg) = read_message(&mut reader) else {
                break;
            };
            let method = msg.get("method").and_then(|m| m.as_str()).unwrap_or("");
            let id = msg.get("id").cloned();
            let params = msg.get("params").cloned().unwrap_or(Json::Null);
            match method {
                "initialize" => {
                    self.on_initialize(&params);
                    if let Some(id) = &id {
                        write_message(&mut writer, &response(id, capabilities()));
                    }
                }
                "initialized" => {}
                "shutdown" => {
                    if let Some(id) = &id {
                        write_message(&mut writer, &response(id, Json::Null));
                    }
                }
                "exit" => break,
                "textDocument/didOpen" => {
                    let uri = params.path(&["textDocument", "uri"]);
                    let text = params.path(&["textDocument", "text"]);
                    if let (Some(uri), Some(text)) =
                        (uri.and_then(|u| u.as_str()), text.and_then(|t| t.as_str()))
                    {
                        let uri = uri.to_string();
                        self.open_docs.insert(uri.clone());
                        self.update_doc(&uri, text.to_string());
                        self.publish_diagnostics(&mut writer, &uri);
                    }
                }
                "textDocument/didChange" => {
                    let uri = params
                        .path(&["textDocument", "uri"])
                        .and_then(|u| u.as_str())
                        .map(|s| s.to_string());
                    let text = params
                        .get("contentChanges")
                        .and_then(|c| c.as_array())
                        .and_then(|a| a.last())
                        .and_then(|c| c.get("text"))
                        .and_then(|t| t.as_str())
                        .map(|s| s.to_string());
                    if let (Some(uri), Some(text)) = (uri, text) {
                        self.update_doc(&uri, text);
                        self.publish_diagnostics(&mut writer, &uri);
                    }
                }
                "textDocument/didClose" => {
                    if let Some(uri) = params
                        .path(&["textDocument", "uri"])
                        .and_then(|u| u.as_str())
                    {
                        self.open_docs.remove(uri);
                    }
                }
                "workspace/didChangeWatchedFiles" => {
                    if let Some(project) = &mut self.project {
                        project.reload_aliases();
                    }
                    let uris: Vec<String> = self.open_docs.iter().cloned().collect();
                    for uri in uris {
                        self.publish_diagnostics(&mut writer, &uri);
                    }
                }
                "textDocument/completion" => {
                    let result = self.on_completion(&params).unwrap_or(Json::Array(vec![]));
                    if let Some(id) = &id {
                        write_message(&mut writer, &response(id, result));
                    }
                }
                "textDocument/hover" => {
                    let result = self.on_hover(&params).unwrap_or(Json::Null);
                    if let Some(id) = &id {
                        write_message(&mut writer, &response(id, result));
                    }
                }
                "textDocument/semanticTokens/full" => {
                    let result = self.on_semantic_tokens(&params).unwrap_or(Json::Null);
                    if let Some(id) = &id {
                        write_message(&mut writer, &response(id, result));
                    }
                }
                "textDocument/inlayHint" => {
                    let result = self.on_inlay_hints(&params).unwrap_or(Json::Array(vec![]));
                    if let Some(id) = &id {
                        write_message(&mut writer, &response(id, result));
                    }
                }
                "workspace/didChangeConfiguration" => {
                    if let Some(settings) = params.path(&["settings", "luar"]) {
                        self.apply_settings(settings);
                    }
                }
                _ => {
                    if let Some(id) = &id {
                        write_message(&mut writer, &response(id, Json::Null));
                    }
                }
            }
        }
    }

    fn on_initialize(&mut self, params: &Json) {
        if let Some(opts) = params.get("initializationOptions") {
            self.apply_settings(opts);
        }
        let root = params
            .get("rootUri")
            .and_then(|u| u.as_str())
            .and_then(uri_to_path)
            .or_else(|| {
                params
                    .get("rootPath")
                    .and_then(|p| p.as_str())
                    .map(|p| PathBuf::from(p))
            });
        if let Some(root) = root {
            self.project = Some(Project::load(&root));
        }
    }

    fn apply_settings(&mut self, opts: &Json) {
        if let Some(v) = opts.get("inlayHints").and_then(|v| v.as_bool()) {
            self.settings.inlay_hints = v;
        }
        if let Some(v) = opts.get("showMutability").and_then(|v| v.as_bool()) {
            self.settings.show_mutability = v;
        }
        if let Some(v) = opts.get("autoImport").and_then(|v| v.as_bool()) {
            self.settings.auto_import = v;
        }
    }

    fn update_doc(&mut self, uri: &str, text: String) {
        let Some(path) = uri_to_path(uri) else {
            return;
        };
        if self.project.is_none() {
            let root = path.parent().map(Path::to_path_buf).unwrap_or_default();
            self.project = Some(Project::load(&root));
        }
        if let Some(project) = &mut self.project {
            project.update_file(&path, text);
        }
    }

    fn publish_diagnostics(&self, writer: &mut impl Write, uri: &str) {
        let Some(path) = uri_to_path(uri) else {
            return;
        };
        let diags: Vec<Json> = self
            .project
            .as_ref()
            .and_then(|p| p.file(&path))
            .map(|info| {
                info.diagnostics
                    .iter()
                    .map(|d| {
                        let line = d.line.saturating_sub(1) as i64;
                        let col = d.col.saturating_sub(1) as i64;
                        let line_len = info
                            .source
                            .lines()
                            .nth(line as usize)
                            .map(|l| l.chars().count() as i64)
                            .unwrap_or(col + 1);
                        Json::obj(vec![
                            (
                                "range",
                                Json::obj(vec![
                                    (
                                        "start",
                                        Json::obj(vec![
                                            ("line", Json::int(line)),
                                            ("character", Json::int(col)),
                                        ]),
                                    ),
                                    (
                                        "end",
                                        Json::obj(vec![
                                            ("line", Json::int(line)),
                                            ("character", Json::int(line_len.max(col + 1))),
                                        ]),
                                    ),
                                ]),
                            ),
                            ("severity", Json::int(d.severity as i64)),
                            ("source", Json::str("luar")),
                            ("message", Json::str(d.message.clone())),
                        ])
                    })
                    .collect()
            })
            .unwrap_or_default();
        let note = Json::obj(vec![
            ("jsonrpc", Json::str("2.0")),
            ("method", Json::str("textDocument/publishDiagnostics")),
            (
                "params",
                Json::obj(vec![
                    ("uri", Json::str(uri)),
                    ("diagnostics", Json::Array(diags)),
                ]),
            ),
        ]);
        write_message(writer, &note);
    }

    fn doc_position(&self, params: &Json) -> Option<(PathBuf, usize, usize)> {
        let uri = params
            .path(&["textDocument", "uri"])
            .and_then(|u| u.as_str())?;
        let path = uri_to_path(uri)?;
        let line = params.path(&["position", "line"])?.as_i64()? as usize;
        let character = params.path(&["position", "character"])?.as_i64()? as usize;
        Some((path, line, character))
    }

    fn on_completion(&self, params: &Json) -> Option<Json> {
        let (path, line, character) = self.doc_position(params)?;
        let project = self.project.as_ref()?;
        let view = FileView::from_project(project, &path)?;
        let text = project.file(&path)?.source.clone();
        let mut items = completion::complete(&view, &text, line, character);
        if !self.settings.auto_import {
            items.retain(|i| i.auto_import.is_none());
        }
        let json_items: Vec<Json> = items
            .into_iter()
            .map(|item| {
                let mut map: Vec<(&str, Json)> = vec![
                    ("label", Json::str(item.label.clone())),
                    ("kind", Json::int(item.kind)),
                    ("detail", Json::str(item.detail)),
                ];
                if let Some(insert) = item.insert_text {
                    map.push(("insertText", Json::str(insert)));
                    if item.is_snippet {
                        map.push(("insertTextFormat", Json::int(2)));
                    }
                }
                if let Some(sort) = item.sort_text {
                    map.push(("sortText", Json::str(sort)));
                }
                let mut edits: Vec<Json> = Vec::new();
                if let Some(ai) = item.auto_import {
                    let pos = |l: u32| {
                        Json::obj(vec![
                            ("line", Json::int(l as i64)),
                            ("character", Json::int(0)),
                        ])
                    };
                    edits.push(Json::obj(vec![
                        (
                            "range",
                            Json::obj(vec![
                                ("start", pos(ai.line0)),
                                ("end", pos(ai.line0)),
                            ]),
                        ),
                        ("newText", Json::str(ai.new_text)),
                    ]));
                }
                if let Some(ee) = item.extra_edit {
                    let pos = |c: u32| {
                        Json::obj(vec![
                            ("line", Json::int(ee.line0 as i64)),
                            ("character", Json::int(c as i64)),
                        ])
                    };
                    edits.push(Json::obj(vec![
                        (
                            "range",
                            Json::obj(vec![
                                ("start", pos(ee.start_col)),
                                ("end", pos(ee.end_col)),
                            ]),
                        ),
                        ("newText", Json::str(ee.new_text)),
                    ]));
                }
                if !edits.is_empty() {
                    map.push(("additionalTextEdits", Json::Array(edits)));
                }
                Json::obj(map)
            })
            .collect();
        Some(Json::Array(json_items))
    }

    fn on_hover(&self, params: &Json) -> Option<Json> {
        let (path, line, character) = self.doc_position(params)?;
        let project = self.project.as_ref()?;
        let info = project.file(&path)?;
        let view = FileView::from_project(project, &path)?;
        let text = &info.source;
        let line_text = text.lines().nth(line)?;
        let (word, word_start, word_end) = completion::word_at(line_text, character)?;
        let cur_line = line as u32 + 1;

        let self_class = completion::enclosing_class(text, line, character);
        let chain = completion::chain_before(line_text, word_end);
        let markdown = if let Some(chain) = chain.filter(|c| {
            !c.segments.is_empty() && c.partial == word && word_start > 0
        }) {
            let recv =
                completion::resolve_chain_full(&view, &chain, cur_line, self_class.as_deref())?;
            let member_ty = view.member_type(&recv, &word);
            if member_ty == Type::Unknown {
                return None;
            }
            let docs = self
                .member_docs(&view, &recv, &word)
                .or_else(|| self.luard_docs(&word));
            hover_markdown(&word, &member_ty, docs)
        } else if word == "self" {
            let class = self_class?;
            format!("```luar\nself: {class}\n```")
        } else if word == "super" {
            let class = self_class?;
            let parent = view.find_class(&class)?.parent.clone()?;
            format!("```luar\nsuper: {parent}\n```")
        } else {
            let binding = view.binding_at(&word, cur_line).cloned().or_else(|| {
                view.analysis.binding(&word).cloned()
            });
            match binding {
                Some(b) => {
                    let docs = doc_comment_above(text, b.line);
                    let header = binding_header(&b);
                    let mut md = format!("```luar\n{header}\n```");
                    if let Some(d) = docs {
                        md.push_str("\n\n---\n\n");
                        md.push_str(&d);
                    }
                    md
                }
                None => {
                    let ty = view.type_of_name(&word, cur_line)?;
                    hover_markdown(&word, &ty, self.luard_docs(&word))
                }
            }
        };

        Some(Json::obj(vec![(
            "contents",
            Json::obj(vec![
                ("kind", Json::str("markdown")),
                ("value", Json::str(markdown)),
            ]),
        )]))
    }

    fn member_docs(
        &self,
        view: &FileView,
        recv: &Type,
        member: &str,
    ) -> Option<String> {
        let class = match recv {
            Type::Instance(c) | Type::Class(c) => c.clone(),
            _ => return None,
        };
        let project = self.project.as_ref()?;
        let mut current = Some(class);
        let mut guard = 0;
        while let Some(c) = current {
            guard += 1;
            if guard > 64 {
                break;
            }
            if let Some(src) = class_source(project, &c) {
                if let Some(line) = find_member_line(&src, &c, member) {
                    if let Some(docs) = doc_comment_above(&src, Some(line)) {
                        return Some(docs);
                    }
                }
            }
            current = view.find_class(&c).and_then(|i| i.parent.clone());
        }
        None
    }

    fn on_semantic_tokens(&self, params: &Json) -> Option<Json> {
        let uri = params
            .path(&["textDocument", "uri"])
            .and_then(|u| u.as_str())?;
        let path = uri_to_path(uri)?;
        let project = self.project.as_ref()?;
        let info = project.file(&path)?;
        let Ok(tokens) = luar::lexer::tokenize(&info.source) else {
            return None;
        };
        let mut data: Vec<Json> = Vec::new();
        let mut prev_line = 0i64;
        let mut prev_col = 0i64;
        let mut after_access = false;
        for t in &tokens {
            if t.kind == luar::lexer::TokenKind::Identifier && !after_access {
                if let Some((_, ty)) = project
                    .luard_globals
                    .iter()
                    .find(|(n, _)| *n == t.text)
                {
                    let token_type = match ty {
                        Type::Function(_) => 0i64,
                        _ => 1i64,
                    };
                    let line0 = t.span.line as i64 - 1;
                    let col0 = t.span.col as i64 - 1;
                    let d_line = line0 - prev_line;
                    let d_col = if d_line == 0 { col0 - prev_col } else { col0 };
                    data.push(Json::int(d_line));
                    data.push(Json::int(d_col));
                    data.push(Json::int(t.text.chars().count() as i64));
                    data.push(Json::int(token_type));
                    data.push(Json::int(1));
                    prev_line = line0;
                    prev_col = col0;
                }
            }
            after_access = t.kind == luar::lexer::TokenKind::Operator
                && (t.text == "." || t.text == ":");
        }
        Some(Json::obj(vec![("data", Json::Array(data))]))
    }

    fn luard_docs(&self, name: &str) -> Option<String> {
        let project = self.project.as_ref()?;
        for p in &project.luard_files {
            let Ok(src) = std::fs::read_to_string(p) else {
                continue;
            };
            if let Some(line) = luard_decl_line(&src, name) {
                if let Some(docs) = doc_comment_above(&src, Some(line)) {
                    return Some(docs);
                }
            }
        }
        None
    }

    fn on_inlay_hints(&self, params: &Json) -> Option<Json> {
        if !self.settings.inlay_hints {
            return Some(Json::Array(vec![]));
        }
        let uri = params
            .path(&["textDocument", "uri"])
            .and_then(|u| u.as_str())?;
        let path = uri_to_path(uri)?;
        let project = self.project.as_ref()?;
        let info = project.file(&path)?;
        let start_line = params.path(&["range", "start", "line"])?.as_i64()? as u32;
        let end_line = params.path(&["range", "end", "line"])?.as_i64()? as u32;
        let lines: Vec<&str> = info.source.lines().collect();

        let mut hints = Vec::new();
        for b in &info.analysis.bindings {
            let Some(line) = b.line else {
                continue;
            };
            if line < start_line + 1 || line > end_line + 1 {
                continue;
            }
            let key = (b.name.clone(), line);
            let annotated = info.annotations.vars.contains_key(&key)
                && !info.annotations.cast_vars.contains(&key);
            if annotated {
                continue;
            }
            let mutability = match b.kind {
                BindingKind::Declare { mutability, .. } => Some(mutability),
                BindingKind::BareAssign => Some(Mutability::Const),
                BindingKind::Buff => Some(Mutability::Mutable),
                _ => continue,
            };
            let Some(line_text) = lines.get(line as usize - 1) else {
                continue;
            };
            let Some(col) = find_name_end(line_text, &b.name) else {
                continue;
            };
            let name_start = col.saturating_sub(b.name.chars().count());
            let prefix: String = line_text.chars().take(name_start).collect();
            if completion::line_has_word(&prefix, "function") {
                continue;
            }
            let label = if self.settings.show_mutability {
                match mutability {
                    Some(Mutability::Const) => format!(": imut {}", b.ty),
                    Some(Mutability::Mutable) => format!(": mut {}", b.ty),
                    None => format!(": {}", b.ty),
                }
            } else {
                format!(": {}", b.ty)
            };
            let label = if label.chars().count() > 48 {
                let mut cut: String = label.chars().take(47).collect();
                cut.push('…');
                cut
            } else {
                label
            };
            hints.push(Json::obj(vec![
                (
                    "position",
                    Json::obj(vec![
                        ("line", Json::int(line as i64 - 1)),
                        ("character", Json::int(col as i64)),
                    ]),
                ),
                ("label", Json::str(label)),
                ("kind", Json::int(1)),
                ("paddingLeft", Json::Bool(false)),
            ]));
        }
        let mut seen_params: std::collections::HashSet<(u32, String)> =
            std::collections::HashSet::new();
        for (line, name, ty) in &info.analysis.param_hints {
            if *line < start_line + 1 || *line > end_line + 3 {
                continue;
            }
            if !seen_params.insert((*line, name.clone())) {
                continue;
            }
            let Some((row, col)) = find_param_position(&lines, *line, name) else {
                continue;
            };
            let label = format!(": {}", compact_type(ty, 0));
            let label = if label.chars().count() > 32 {
                let mut cut: String = label.chars().take(31).collect();
                cut.push('…');
                cut
            } else {
                label
            };
            hints.push(Json::obj(vec![
                (
                    "position",
                    Json::obj(vec![
                        ("line", Json::int(row as i64)),
                        ("character", Json::int(col as i64)),
                    ]),
                ),
                ("label", Json::str(label)),
                ("kind", Json::int(1)),
                ("paddingLeft", Json::Bool(false)),
            ]));
        }
        Some(Json::Array(hints))
    }
}

fn capabilities() -> Json {
    Json::obj(vec![(
        "capabilities",
        Json::obj(vec![
            ("textDocumentSync", Json::int(1)),
            (
                "completionProvider",
                Json::obj(vec![(
                    "triggerCharacters",
                    Json::Array(
                        [".", ":", "\"", "'", "@", "/", "#"]
                            .iter()
                            .map(|s| Json::str(*s))
                            .collect(),
                    ),
                )]),
            ),
            ("hoverProvider", Json::Bool(true)),
            ("inlayHintProvider", Json::Bool(true)),
            (
                "semanticTokensProvider",
                Json::obj(vec![
                    (
                        "legend",
                        Json::obj(vec![
                            (
                                "tokenTypes",
                                Json::Array(vec![
                                    Json::str("function"),
                                    Json::str("variable"),
                                ]),
                            ),
                            (
                                "tokenModifiers",
                                Json::Array(vec![Json::str("defaultLibrary")]),
                            ),
                        ]),
                    ),
                    ("full", Json::Bool(true)),
                ]),
            ),
        ]),
    )])
}

fn find_param_position(lines: &[&str], line1: u32, name: &str) -> Option<(usize, usize)> {
    if line1 == 0 {
        return None;
    }
    let start = line1 as usize - 1;
    for row in start..(start + 5).min(lines.len()) {
        let text = lines[row];
        let Some(end) = find_name_end(text, name) else {
            continue;
        };
        let after: String = text.chars().skip(end).collect();
        if after.trim_start().starts_with(':') {
            return None;
        }
        return Some((row, end));
    }
    None
}

fn compact_type(ty: &crate::types::Type, depth: usize) -> String {
    use crate::types::Type;
    match ty {
        Type::Table(tt) => {
            if let Some(n) = &tt.name {
                return n.clone();
            }
            if let Some(elem) = &tt.array {
                if depth < 2 {
                    return format!("{{{}}}", compact_type(elem, depth + 1));
                }
                return "{…}".into();
            }
            if tt.fields.is_empty() {
                return "{}".into();
            }
            "{…}".into()
        }
        Type::Function(_) => "function".into(),
        Type::Union(parts) => {
            let inner: Vec<String> = parts
                .iter()
                .map(|p| compact_type(p, depth + 1))
                .collect();
            inner.join(" | ")
        }
        other => other.to_string(),
    }
}

fn find_name_end(line: &str, name: &str) -> Option<usize> {
    let chars: Vec<char> = line.chars().collect();
    let name_chars: Vec<char> = name.chars().collect();
    let n = name_chars.len();
    if n == 0 {
        return None;
    }
    let mut i = 0;
    while i + n <= chars.len() {
        if chars[i..i + n] == name_chars[..] {
            let before_ok = i == 0 || !(chars[i - 1].is_alphanumeric() || chars[i - 1] == '_');
            let after = i + n;
            let after_ok = after >= chars.len()
                || !(chars[after].is_alphanumeric() || chars[after] == '_');
            if before_ok && after_ok {
                return Some(after);
            }
        }
        i += 1;
    }
    None
}

fn binding_header(b: &crate::infer::Binding) -> String {
    let name = &b.name;
    match (&b.kind, &b.ty) {
        (_, Type::Function(Some(sig))) => format!("function {name}{sig}"),
        (BindingKind::Class, _) => format!("class {name}"),
        (BindingKind::Enum, _) => format!("enum {name}"),
        (BindingKind::Interface, _) => format!("interface {name}"),
        (BindingKind::Declare { mutability, .. }, ty) => {
            let kw = match mutability {
                Mutability::Const => "const",
                Mutability::Mutable => "local",
            };
            format!("{kw} {name}: {ty}")
        }
        (BindingKind::Buff, ty) => format!("buff {name}: {ty}"),
        (_, ty) => format!("{name}: {ty}"),
    }
}

fn hover_markdown(name: &str, ty: &Type, docs: Option<String>) -> String {
    let header = match ty {
        Type::Function(Some(sig)) => format!("function {name}{sig}"),
        other => format!("{name}: {other}"),
    };
    let mut md = format!("```luar\n{header}\n```");
    if let Some(d) = docs {
        md.push_str("\n\n---\n\n");
        md.push_str(&d);
    }
    md
}

fn class_source(project: &Project, class: &str) -> Option<String> {
    for m in project.files.values() {
        if m.analysis.classes.contains_key(class) {
            return Some(m.source.clone());
        }
    }
    for p in &project.luard_files {
        if let Ok(s) = std::fs::read_to_string(p) {
            if find_class_header(&s, class).is_some() {
                return Some(s);
            }
        }
    }
    None
}

fn find_class_header(src: &str, class: &str) -> Option<usize> {
    src.lines().position(|line| {
        completion::line_has_word(line, "class") && completion::line_has_word(line, class)
    })
}

const MEMBER_DECL_STARTS: [&str; 10] = [
    "public", "private", "protected", "static", "abstract", "final", "override", "function",
    "get", "set",
];

fn find_member_line(src: &str, class: &str, member: &str) -> Option<u32> {
    let header = find_class_header(src, class)?;
    let mut depth: i32 = 0;
    let mut opened = false;
    for (i, raw) in src.lines().enumerate().skip(header) {
        let cleaned = completion::strip_strings_and_comments(raw);
        if opened && depth > 0 {
            let t = cleaned.trim_start();
            let decl_start = MEMBER_DECL_STARTS.iter().any(|k| {
                t.starts_with(k)
                    && t[k.len()..]
                        .chars()
                        .next()
                        .map(|c| c == ' ' || c == '\t')
                        .unwrap_or(false)
            });
            if decl_start && completion::line_has_word(&cleaned, member) {
                return Some(i as u32 + 1);
            }
        }
        for c in cleaned.chars() {
            match c {
                '{' => {
                    depth += 1;
                    opened = true;
                }
                '}' => depth -= 1,
                _ => {}
            }
        }
        if opened && depth <= 0 {
            break;
        }
    }
    None
}

fn luard_decl_line(src: &str, name: &str) -> Option<u32> {
    for (i, raw) in src.lines().enumerate() {
        let line = raw.trim_start();
        if line.starts_with("--") {
            continue;
        }
        let mut search = 0;
        while let Some(pos) = line[search..].find(name) {
            let start = search + pos;
            let end = start + name.len();
            search = end;
            let before = line[..start].chars().last();
            let boundary_before = before
                .map(|c| !(c.is_alphanumeric() || c == '_'))
                .unwrap_or(true);
            let after = line[end..].trim_start();
            let boundary_after = line[end..]
                .chars()
                .next()
                .map(|c| !(c.is_alphanumeric() || c == '_'))
                .unwrap_or(true);
            if !(boundary_before && boundary_after) {
                continue;
            }
            let declares = (after.starts_with('=') && !after.starts_with("=="))
                || after.starts_with('(')
                || after.starts_with(':')
                || line.starts_with(&format!("function {name}"))
                || line.starts_with(&format!("class {name}"))
                || line.starts_with(&format!("enum {name}"));
            if declares {
                return Some(i as u32 + 1);
            }
        }
    }
    None
}

pub fn doc_comment_above(text: &str, line: Option<u32>) -> Option<String> {
    let line = line? as usize;
    if line < 2 {
        return None;
    }
    let lines: Vec<&str> = text.lines().collect();
    let mut collected: Vec<String> = Vec::new();
    let mut i = line - 1;
    while i > 0 {
        let candidate = lines.get(i - 1)?.trim();
        if candidate.starts_with("--#") {
            break;
        }
        if let Some(body) = candidate.strip_prefix("--") {
            let body = body
                .trim_start_matches('-')
                .trim_start_matches("[[")
                .trim_end_matches("]]")
                .trim();
            collected.push(body.to_string());
            i -= 1;
        } else {
            break;
        }
    }
    if collected.is_empty() {
        return None;
    }
    collected.reverse();
    Some(collected.join("\n\n"))
}

pub fn run_server() {
    let mut server = Server::default();
    server.run();
}

pub fn settings_to_json(s: &Settings) -> Json {
    let mut map = BTreeMap::new();
    map.insert("inlayHints".to_string(), Json::Bool(s.inlay_hints));
    map.insert("showMutability".to_string(), Json::Bool(s.show_mutability));
    map.insert("autoImport".to_string(), Json::Bool(s.auto_import));
    Json::Object(map)
}
