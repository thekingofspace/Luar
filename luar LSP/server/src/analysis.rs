
use std::collections::{HashMap, HashSet};

use luar::ast::{ClassMember, Expr, Stmt};

#[derive(Clone)]
pub struct Member {
    pub name: String,
    pub is_method: bool,
    pub detail: String,
}

#[derive(Clone)]
pub struct ClassData {
    pub name: String,
    pub parent: Option<String>,
    pub mixins: Vec<String>,
    pub members: Vec<Member>,
}

#[derive(Clone)]
pub struct VarInfo {

    pub ty: String,

    pub mutable: bool,

    pub global: bool,

    pub literals: Vec<String>,
}

#[derive(Default, Clone)]
pub struct FileAnalysis {
    pub classes: Vec<ClassData>,
    pub interfaces: Vec<String>,
    pub aliases: Vec<String>,

    pub alias_targets: HashMap<String, String>,

    pub vars: HashMap<String, VarInfo>,

    pub functions: HashMap<String, String>,

    pub class_ranges: Vec<(String, u32, u32)>,

    pub enums: HashMap<String, EnumInfo>,

    pub module_vars: HashMap<String, String>,

    pub docs: HashMap<String, String>,
}

#[derive(Clone, Default)]
pub struct EnumInfo {
    pub variants: Vec<String>,
    pub global: bool,
}

const MODS: &[&str] = &[
    "pub", "local", "const", "public", "private", "protected", "static", "abstract", "final",
    "override", "export",
];
const PRIMS: &[&str] = &[
    "boolean", "number", "string", "table", "thread", "nil", "any", "unknown", "never", "void",
    "function", "true", "false",
];

pub fn is_primitive(name: &str) -> bool {
    PRIMS.contains(&name)
}

pub fn analyze_file(text: &str) -> Option<FileAnalysis> {
    let stmts = luar::parse_source(text).ok()?;
    let mut fa = FileAnalysis::default();
    for s in &stmts {
        collect(s, &mut fa);
    }
    scan_annotations(text, &mut fa.vars);
    scan_functions(text, &mut fa.functions);
    scan_aliases(text, &mut fa.alias_targets);
    fa.class_ranges = brace_class_ranges(text);
    fa.docs = scan_docs(text);
    inject_prelude_classes(&mut fa);
    Some(fa)
}

pub fn scan_docs(text: &str) -> HashMap<String, String> {
    let mut docs = HashMap::new();
    let mut pending: Vec<String> = Vec::new();
    let mut in_block = false;
    let mut block: Vec<String> = Vec::new();
    for raw in text.lines() {
        let t = raw.trim_start();
        if in_block {
            if let Some(idx) = t.find("]]") {
                block.push(t[..idx].to_string());
                pending = std::mem::take(&mut block);
                in_block = false;
            } else {
                block.push(t.to_string());
            }
            continue;
        }
        if let Some(rest) = t.strip_prefix("--[[") {
            if let Some(idx) = rest.find("]]") {
                pending = vec![rest[..idx].to_string()];
            } else {
                in_block = true;
                block = vec![rest.to_string()];
            }
            continue;
        }
        if t.starts_with("--") && !t.starts_with("--#") {
            let c = t.trim_start_matches('-');
            pending.push(c.strip_prefix(' ').unwrap_or(c).to_string());
            continue;
        }
        if t.is_empty() {
            continue;
        }
        if !pending.is_empty() {
            let rendered = render_doc(&pending);
            if !rendered.is_empty() {
                for name in decl_names(t) {
                    docs.entry(name).or_insert_with(|| rendered.clone());
                }
            }
        }
        pending.clear();
    }
    docs
}

fn render_doc(lines: &[String]) -> String {
    let joined = lines.join("\n");
    let trimmed = joined.trim();
    trimmed
        .lines()
        .map(|l| {
            let lt = l.trim_start();
            if let Some(after_at) = lt.strip_prefix('@') {
                let tag: String = after_at.chars().take_while(|c| !c.is_whitespace()).collect();
                let rest = &lt[1 + tag.len()..];
                format!("**@{tag}**{rest}")
            } else {
                l.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn decl_names(line: &str) -> Vec<String> {
    let s = strip_mods(line);
    if let Some(r) = s.strip_prefix("function") {
        if r.starts_with(|c: char| c.is_whitespace()) {
            let path: String = r
                .trim_start()
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '.' || *c == ':')
                .collect();
            let last = path.split(['.', ':']).last().unwrap_or("").to_string();
            return if last.is_empty() { Vec::new() } else { vec![last] };
        }
    }
    for kw in ["class", "enum", "interface"] {
        if let Some(r) = s.strip_prefix(kw) {
            if r.starts_with(|c: char| c.is_whitespace()) {
                if let Some((n, _)) = read_ident(r.trim_start()) {
                    return vec![n.to_string()];
                }
            }
        }
    }
    if let Some(r) = s.strip_prefix("type ") {
        if let Some((n, _)) = read_ident(r.trim_start()) {
            return vec![n.to_string()];
        }
    }
    if let Some((n, after)) = read_ident(s) {
        let a = after.trim_start();
        if a.starts_with('.') {
            let field: String = a[1..].chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
            return if field.is_empty() { Vec::new() } else { vec![field] };
        }
        if (a.starts_with(':') && !a.starts_with("::")) || a.starts_with('=') || a.starts_with(',') {
            let lhs = s.split('=').next().unwrap_or(s);
            let mut names = Vec::new();
            for part in lhs.split(',') {
                let nm: String = part.trim().chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
                if !nm.is_empty() {
                    names.push(nm);
                }
            }
            return names;
        }
        let _ = n;
    }
    Vec::new()
}

fn inject_prelude_classes(fa: &mut FileAnalysis) {
    if fa.classes.iter().any(|c| c.name == "MonoBehaviour") {
        return;
    }
    let hook = |n: &str| Member { name: n.into(), is_method: true, detail: "lifecycle hook".into() };
    fa.classes.push(ClassData {
        name: "MonoBehaviour".into(),
        parent: None,
        mixins: Vec::new(),
        members: vec![hook("Awake"), hook("Start"), hook("Update"), hook("OnDestroy")],
    });
}

fn scan_aliases(text: &str, targets: &mut HashMap<String, String>) {
    for line in text.lines() {
        let mut t = line.trim_start();
        for kw in ["export ", "pub "] {
            t = t.strip_prefix(kw).map(str::trim_start).unwrap_or(t);
        }
        let Some(rest) = t.strip_prefix("type ") else { continue };
        let Some((name, after)) = read_ident(rest.trim_start()) else { continue };
        if let Some(eq) = after.find('=') {
            let target = resolve_return_type(after[eq + 1..].trim());
            if !target.is_empty() {
                targets.insert(name.to_string(), target);
            }
        }
    }
}

pub fn line_scan(text: &str) -> FileAnalysis {
    let mut fa = FileAnalysis::default();
    scan_vars_lines(text, &mut fa.vars);
    scan_annotations(text, &mut fa.vars);
    scan_functions(text, &mut fa.functions);
    scan_aliases(text, &mut fa.alias_targets);
    fa.classes = scan_classes_lines(text);
    fa.enums = scan_enums_lines(text);
    scan_module_vars_lines(text, &mut fa.module_vars);
    fa.class_ranges = brace_class_ranges(text);
    fa.docs = scan_docs(text);
    inject_prelude_classes(&mut fa);
    fa
}

pub fn merge_classes(cached: &mut Vec<ClassData>, scanned: Vec<ClassData>) {
    for sc in scanned {
        match cached.iter_mut().find(|c| c.name == sc.name) {
            Some(c) => {
                c.parent = sc.parent;
                c.mixins = sc.mixins;
                for m in sc.members {
                    if !c.members.iter().any(|x| x.name == m.name) {
                        c.members.push(m);
                    }
                }
            }
            None => cached.push(sc),
        }
    }
}

fn scan_classes_lines(text: &str) -> Vec<ClassData> {
    let lines: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let s = strip_mods(lines[i].trim_start());
        let Some(rest) = s.strip_prefix("class") else {
            i += 1;
            continue;
        };
        if !rest.starts_with(|c: char| c.is_whitespace()) {
            i += 1;
            continue;
        }
        let Some((name, after)) = read_ident(rest.trim_start()) else {
            i += 1;
            continue;
        };
        let (parent, mixins) = parse_class_header(after);

        let (mut depth, mut seen, mut j) = (0i32, false, i);
        let mut members = Vec::new();
        while j < lines.len() {
            if seen {
                if let Some(m) = scan_member_line(lines[j]) {
                    members.push(m);
                }
            }
            for c in lines[j].chars() {
                if c == '{' {
                    depth += 1;
                    seen = true;
                } else if c == '}' {
                    depth -= 1;
                }
            }
            if seen && depth <= 0 {
                break;
            }
            j += 1;
        }
        out.push(ClassData { name: name.to_string(), parent, mixins, members });
        i = j + 1;
    }
    out
}

pub fn mark_ambient(fa: &mut FileAnalysis) {
    for vi in fa.vars.values_mut() {
        vi.global = true;
    }
    for ei in fa.enums.values_mut() {
        ei.global = true;
    }
}

pub fn visible_enums(files: &[&FileAnalysis], current: &FileAnalysis) -> HashMap<String, Vec<String>> {
    let mut out: HashMap<String, Vec<String>> = HashMap::new();
    let mut add = |name: &String, info: &EnumInfo| {
        let e: &mut Vec<String> = out.entry(name.clone()).or_default();
        for v in &info.variants {
            if !e.contains(v) {
                e.push(v.clone());
            }
        }
    };
    for (n, info) in &current.enums {
        add(n, info);
    }
    for f in files {
        for (n, info) in &f.enums {
            if info.global {
                add(n, info);
            }
        }
    }
    out
}

pub fn module_table_exports(text: &str) -> Vec<String> {

    let mut ret: Option<String> = None;
    for line in text.lines() {
        if let Some(r) = line.trim_start().strip_prefix("return ") {
            if let Some((id, rest)) = read_ident(r.trim_start()) {
                if rest.trim().is_empty() {
                    ret = Some(id.to_string());
                }
            }
        }
    }
    let Some(ret) = ret else { return Vec::new() };
    let prefix = format!("{ret}.");
    let mut out = Vec::new();
    let mut add = |field: &str| {
        let f = field.to_string();
        if !f.is_empty() && !out.contains(&f) {
            out.push(f);
        }
    };
    for line in text.lines() {
        let t = line.trim_start();

        if let Some(r) = strip_mods(t).strip_prefix("function") {
            if let Some(rest) = r.trim_start().strip_prefix(&prefix) {
                if let Some((field, _)) = read_ident(rest) {
                    add(field);
                }
            }
        }

        if let Some(rest) = t.strip_prefix(&prefix) {
            if let Some((field, after)) = read_ident(rest) {
                if after.trim_start().starts_with('=') {
                    add(field);
                }
            }
        }
    }
    out
}

fn scan_module_vars_lines(text: &str, out: &mut HashMap<String, String>) {
    for line in text.lines() {
        let t = strip_mods(line.trim_start());
        let Some((name, after)) = read_ident(t) else { continue };
        let Some(rhs) = after.trim_start().strip_prefix('=') else { continue };
        let Some(rest) = rhs.trim_start().strip_prefix("require") else { continue };
        let Some(rest) = rest.trim_start().strip_prefix('(') else { continue };
        let rest = rest.trim_start();
        let Some(q) = rest.chars().next() else { continue };
        if q == '"' || q == '\'' {
            if let Some(end) = rest[q.len_utf8()..].find(q) {
                out.insert(name.to_string(), rest[q.len_utf8()..q.len_utf8() + end].to_string());
            }
        }
    }
}

fn scan_enums_lines(text: &str) -> HashMap<String, EnumInfo> {
    let lines: Vec<&str> = text.lines().collect();
    let mut out: HashMap<String, EnumInfo> = HashMap::new();
    let mut i = 0;
    while i < lines.len() {
        let trimmed = lines[i].trim_start();
        let global = trimmed.starts_with("pub ") || trimmed.starts_with("export ");
        let s = strip_mods(trimmed);
        let Some(rest) = s.strip_prefix("enum") else {
            i += 1;
            continue;
        };
        if !rest.starts_with(|c: char| c.is_whitespace()) {
            i += 1;
            continue;
        }
        let Some((name, _)) = read_ident(rest.trim_start()) else {
            i += 1;
            continue;
        };

        let (mut depth, mut seen, mut j) = (0i32, false, i);
        let mut body = String::new();
        while j < lines.len() {
            for c in lines[j].chars() {
                match c {
                    '{' => {
                        depth += 1;
                        seen = true;
                    }
                    '}' => depth -= 1,
                    _ if seen && depth >= 1 => body.push(c),
                    _ => {}
                }
            }
            body.push('\n');
            if seen && depth <= 0 {
                break;
            }
            j += 1;
        }
        let entry = out.entry(name.to_string()).or_default();
        entry.global |= global;
        for raw in body.split([',', '\n', ';']) {
            let name_part = raw.split('=').next().unwrap_or(raw);
            for w in name_part.split_whitespace() {
                let id: String = w.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
                if !id.is_empty()
                    && id.starts_with(|c: char| c.is_alphabetic() || c == '_')
                    && !entry.variants.contains(&id)
                {
                    entry.variants.push(id);
                }
            }
        }
        i = j + 1;
    }
    out
}

fn parse_class_header(after: &str) -> (Option<String>, Vec<String>) {
    let head = after.split('{').next().unwrap_or(after);
    let parent = find_word(head, "extends")
        .and_then(|p| read_ident(head[p + "extends".len()..].trim_start()))
        .map(|(id, _)| id.to_string());
    let mut mixins = Vec::new();
    if let Some(p) = find_word(head, "mixin") {
        let mut rest = head[p + "mixin".len()..].trim_start();
        while let Some((id, after)) = read_ident(rest) {
            if id == "implements" {
                break;
            }
            mixins.push(id.to_string());
            match after.trim_start().strip_prefix(',') {
                Some(a) => rest = a.trim_start(),
                None => break,
            }
        }
    }
    (parent, mixins)
}

fn scan_member_line(line: &str) -> Option<Member> {
    let t = strip_mods(line.trim_start());
    if let Some(rest) = t.strip_prefix("function") {
        if rest.starts_with(|c: char| c.is_whitespace()) {
            if let Some((name, _)) = read_ident(rest.trim_start()) {
                return Some(Member { name: name.to_string(), is_method: true, detail: "method".into() });
            }
        }
    }
    for kw in ["get", "set"] {
        if let Some(rest) = t.strip_prefix(kw) {
            if rest.starts_with(|c: char| c.is_whitespace()) {
                if let Some((name, _)) = read_ident(rest.trim_start()) {
                    return Some(Member { name: name.to_string(), is_method: false, detail: "property".into() });
                }
            }
        }
    }

    if let Some((name, after)) = read_ident(t) {
        let a = after.trim_start();
        if (a.starts_with(':') && !a.starts_with("::")) || a.starts_with('=') {
            return Some(Member { name: name.to_string(), is_method: false, detail: "field".into() });
        }
    }
    None
}

fn scan_vars_lines(text: &str, vars: &mut HashMap<String, VarInfo>) {
    let param = |vars: &mut HashMap<String, VarInfo>, n: &str| {
        if !n.is_empty() {
            vars.entry(n.to_string())
                .or_insert(VarInfo { ty: String::new(), mutable: true, global: false, literals: Vec::new() });
        }
    };
    for line in text.lines() {
        let t = line.trim_start();

        if let Some(fpos) = find_word(line, "function") {
            let rest = &line[fpos + "function".len()..];
            if let (Some(open), Some(close)) = (rest.find('('), rest.find(')')) {
                if open < close {
                    for p in rest[open + 1..close].split(',') {
                        param(vars, &p.trim().chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect::<String>());
                    }
                }
            }
        }

        if let Some(r) = t.strip_prefix("for ") {
            let head = r.split(" in ").next().unwrap_or(r).split('=').next().unwrap_or(r);
            for v in head.split(',') {
                param(vars, &v.trim().chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect::<String>());
            }
        }

        let mut rest = t;
        let mut mutable = false;
        let mut global = false;
        let mut is_decl = false;
        if let Some(r) = rest.strip_prefix("pub ") {
            global = true;
            is_decl = true;
            rest = r.trim_start();
        }
        if let Some(r) = rest.strip_prefix("local ") {
            mutable = true;
            is_decl = true;
            rest = r.trim_start();
        } else if let Some(r) = rest.strip_prefix("const ") {
            is_decl = true;
            rest = r.trim_start();
        }
        if !is_decl
            || rest.starts_with("function")
            || rest.starts_with("class")
            || rest.starts_with("interface")
            || rest.starts_with("enum")
            || rest.starts_with("type ")
        {
            continue;
        }
        let lhs = rest.split('=').next().unwrap_or(rest);
        let rhs = rest.splitn(2, '=').nth(1).unwrap_or("").trim();
        for (idx, part) in lhs.split(',').enumerate() {
            let name: String = part.trim().chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
            if name.is_empty() {
                continue;
            }
            let ty = if let Some(colon) = part.find(':') {
                part[colon + 1..].split('=').next().unwrap_or("").trim().chars().take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '.').collect()
            } else if idx == 0 {
                infer_rhs(rhs)
            } else {
                String::new()
            };
            vars.entry(name).or_insert(VarInfo { ty, mutable, global, literals: Vec::new() });
        }
    }
}

fn infer_rhs(rhs: &str) -> String {
    let r = rhs.trim();
    if r.starts_with('"') || r.starts_with('\'') || r.starts_with('`') {
        "string".into()
    } else if r.starts_with("true") || r.starts_with("false") {
        "boolean".into()
    } else if r.starts_with('-') || r.starts_with(|c: char| c.is_ascii_digit()) {
        "number".into()
    } else if r.starts_with('{') {
        "table".into()
    } else if r.starts_with("function") {
        "function".into()
    } else {

        let name: String = r.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
        if r[name.len()..].trim_start().starts_with('(') {
            if name == "require" { "module".into() } else { name }
        } else {
            String::new()
        }
    }
}

fn scan_functions(text: &str, funcs: &mut HashMap<String, String>) {
    for line in text.lines() {
        let Some(fpos) = find_word(line, "function") else { continue };
        let after = line[fpos + "function".len()..].trim_start();
        let Some((name, rest)) = read_ident(after) else { continue };

        let Some(paren) = rest.find(')') else { continue };
        let tail = rest[paren + 1..].trim_start();
        if let Some(rt) = tail.strip_prefix(':') {
            let resolved = resolve_return_type(rt.trim_start());
            if !resolved.is_empty() {
                funcs.insert(name.to_string(), resolved);
            }
        }
    }
}

pub const TYPE_FUNCS: &[&str] = &[
    "classof", "typeof", "returnof", "valueof", "elementof", "instanceof", "awaited",
    "nonnil", "nonnull", "readonly", "writable", "mutable", "partial", "required",
    "optional", "deep", "shallow", "unwrap", "paramsof", "default", "valuefrom",
];

pub const KEY_TYPE_FUNCS: &[&str] = &["keyof", "nameof", "indexof"];

fn resolve_return_type(s: &str) -> String {
    let s = s.trim();
    if let Some(lt) = s.find('<') {
        let name: String = s[..lt].chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
        if !name.is_empty() {
            if KEY_TYPE_FUNCS.contains(&name.as_str()) {
                return "string".into();
            }
            if TYPE_FUNCS.contains(&name.as_str()) {
                let inner = s[lt + 1..]
                    .trim()
                    .trim_end_matches('>')
                    .trim()
                    .trim_start_matches('{')
                    .trim_end_matches('}')
                    .trim();
                return resolve_return_type(inner);
            }
        }
    }
    read_type_name(s)
}

fn find_word(line: &str, word: &str) -> Option<usize> {
    let mut from = 0;
    while let Some(rel) = line[from..].find(word) {
        let i = from + rel;
        let left = i == 0 || !is_ident_byte(line.as_bytes()[i - 1]);
        let after = i + word.len();
        let right = after >= line.len() || !is_ident_byte(line.as_bytes()[after]);
        if left && right {
            return Some(i);
        }
        from = i + word.len();
    }
    None
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn collect(s: &Stmt, fa: &mut FileAnalysis) {
    match s {
        Stmt::Class { name, parent, mixins, members, .. } => {
            let mut ms = Vec::new();
            for m in members {
                match m {
                    ClassMember::Field { name, .. } => {
                        ms.push(Member { name: name.clone(), is_method: false, detail: "field".into() });
                    }
                    ClassMember::Method { name, func, .. } => {
                        ms.push(Member { name: name.clone(), is_method: true, detail: format!("({})", func.params.join(", ")) });
                    }
                    ClassMember::Getter { name, .. } | ClassMember::Setter { name, .. } => {
                        if !ms.iter().any(|x| &x.name == name) {
                            ms.push(Member { name: name.clone(), is_method: false, detail: "property".into() });
                        }
                    }
                    ClassMember::Constructor { .. } | ClassMember::Operator { .. } => {}
                }

                if let Some(func) = member_body(m) {
                    add_params(fa, &func.params);
                    func.body.iter().for_each(|x| collect(x, fa));
                }
            }
            fa.classes.push(ClassData { name: name.clone(), parent: parent.clone(), mixins: mixins.clone(), members: ms });
        }
        Stmt::Interface { name, .. } => fa.interfaces.push(name.clone()),
        Stmt::TypeAlias { name, .. } => fa.aliases.push(name.clone()),
        Stmt::Enum { visibility, name, variants, .. } => {

            let global = matches!(visibility, luar::ast::Visibility::Pub);
            let entry = fa.enums.entry(name.clone()).or_default();
            entry.global |= global;
            for (v, _) in variants {
                if !entry.variants.contains(v) {
                    entry.variants.push(v.clone());
                }
            }
        }
        Stmt::Declare { names, inits, visibility, mutability, .. } => {
            let mutable = matches!(mutability, luar::ast::Mutability::Mutable);
            let global = matches!(visibility, luar::ast::Visibility::Pub);
            for (i, n) in names.iter().enumerate() {
                let ty = inits.get(i).and_then(infer).unwrap_or_default();
                if let Some(path) = inits.get(i).and_then(require_path) {
                    fa.module_vars.insert(n.clone(), path);
                }
                fa.vars.insert(n.clone(), VarInfo { ty, mutable, global, literals: Vec::new() });
            }

            inits.iter().for_each(|e| collect_expr(e, fa));
        }

        Stmt::Do(b) => b.iter().for_each(|x| collect(x, fa)),
        Stmt::If { branches, else_block, .. } => {
            for (cond, b) in branches {
                collect_expr(cond, fa);
                b.iter().for_each(|x| collect(x, fa));
            }
            if let Some(b) = else_block {
                b.iter().for_each(|x| collect(x, fa));
            }
        }
        Stmt::While { cond, body, .. } => {
            collect_expr(cond, fa);
            body.iter().for_each(|x| collect(x, fa));
        }
        Stmt::ForNumeric { body, .. } | Stmt::ForIn { body, .. } => {
            body.iter().for_each(|x| collect(x, fa));
        }
        Stmt::Return { values, .. } => values.iter().for_each(|e| collect_expr(e, fa)),
        Stmt::Expr(e, _) => collect_expr(e, fa),
        _ => {}
    }
}

fn member_body(m: &ClassMember) -> Option<&luar::ast::FnBody> {
    match m {
        ClassMember::Method { func, .. }
        | ClassMember::Constructor { func }
        | ClassMember::Operator { func, .. }
        | ClassMember::Getter { func, .. }
        | ClassMember::Setter { func, .. } => Some(func),
        ClassMember::Field { .. } => None,
    }
}

fn add_params(fa: &mut FileAnalysis, params: &[String]) {
    for p in params {
        fa.vars
            .entry(p.clone())
            .or_insert(VarInfo { ty: String::new(), mutable: true, global: false, literals: Vec::new() });
    }
}

fn collect_expr(e: &Expr, fa: &mut FileAnalysis) {
    match e {
        Expr::Function { params, body, .. } => {
            add_params(fa, params);
            body.iter().for_each(|x| collect(x, fa));
        }
        Expr::Call { callee, args } => {
            collect_expr(callee, fa);
            args.iter().for_each(|a| collect_expr(a, fa));
        }
        Expr::MethodCall { receiver, args, .. } => {
            collect_expr(receiver, fa);
            args.iter().for_each(|a| collect_expr(a, fa));
        }
        Expr::Binary { lhs, rhs, .. } | Expr::Logical { lhs, rhs, .. } => {
            collect_expr(lhs, fa);
            collect_expr(rhs, fa);
        }
        Expr::Unary { expr, .. } => collect_expr(expr, fa),
        Expr::Index { base, key } => {
            collect_expr(base, fa);
            collect_expr(key, fa);
        }
        Expr::Table(entries) => {
            for entry in entries {
                match entry {
                    luar::ast::TableEntry::Positional(v) => collect_expr(v, fa),
                    luar::ast::TableEntry::Keyed { key, value } => {
                        collect_expr(key, fa);
                        collect_expr(value, fa);
                    }
                }
            }
        }
        Expr::Switch { subject, cases, default } => {
            collect_expr(subject, fa);
            for c in cases {
                c.body.iter().for_each(|x| collect(x, fa));
            }
            if let Some(b) = default {
                b.iter().for_each(|x| collect(x, fa));
            }
        }
        _ => {}
    }
}

fn infer(e: &Expr) -> Option<String> {
    Some(match e {
        Expr::Bool(_) => "boolean".into(),
        Expr::Int(_) | Expr::Float(_) => "number".into(),
        Expr::Str(_) => "string".into(),
        Expr::Nil => "nil".into(),
        Expr::Table(_) => "table".into(),
        Expr::Function { .. } => "function".into(),
        Expr::Call { callee, .. } => match &**callee {
            Expr::Name(n) if n == "require" => "module".into(),
            Expr::Name(n) => n.clone(),
            _ => return None,
        },
        _ => return None,
    })
}

fn require_path(e: &Expr) -> Option<String> {
    if let Expr::Call { callee, args } = e {
        if matches!(&**callee, Expr::Name(n) if n == "require") {
            if let Some(Expr::Str(p)) = args.first() {
                return Some(p.clone());
            }
        }
    }
    None
}

fn scan_annotations(text: &str, vars: &mut HashMap<String, VarInfo>) {
    let set = |vars: &mut HashMap<String, VarInfo>, line: &str, name: &str, ty: String, lits: Vec<String>| {
        if let Some(vi) = vars.get_mut(name) {
            if !ty.is_empty() {
                vi.ty = ty;
            }
            if !lits.is_empty() {
                vi.literals = lits;
            }
        } else {
            let t = line.trim_start();
            vars.insert(
                name.to_string(),
                VarInfo { ty, mutable: t.contains("local"), global: t.starts_with("pub"), literals: lits },
            );
        }
    };
    for line in text.lines() {

        let mut annotated = false;
        let stripped = strip_mods(line.trim_start());
        if let Some((name, rest)) = read_ident(stripped) {
            let rest = rest.trim_start();
            if rest.starts_with(':') && !rest.starts_with("::") && !MODS.contains(&name) {
                let expr = rest[1..].split('=').next().unwrap_or("");
                let lits = string_literals(expr);
                let ty = resolve_return_type(expr.trim());
                let ty = if ty.is_empty() && !lits.is_empty() { "string".into() } else { ty };
                if !ty.is_empty() || !lits.is_empty() {
                    set(vars, line, name, ty, lits);
                    annotated = true;
                }
            }
        }

        if !annotated {
            if let Some(idx) = line.find("::") {
                if let Some((name, _)) = read_ident(strip_mods(line.trim_start())) {
                    if !MODS.contains(&name) {
                        let expr = &line[idx + 2..];
                        let lits = string_literals(expr);
                        let ty = read_type_name(expr.trim_start());
                        let ty = if ty.is_empty() && !lits.is_empty() { "string".into() } else { ty };
                        if !ty.is_empty() || !lits.is_empty() {
                            set(vars, line, name, ty, lits);
                        }
                    }
                }
            }
        }
    }
}

fn string_literals(s: &str) -> Vec<String> {
    let chars: Vec<char> = s.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '"' || c == '\'' {
            i += 1;
            let start = i;
            while i < chars.len() && chars[i] != c {
                i += 1;
            }
            out.push(chars[start..i].iter().collect());
            i += 1;
        } else {
            i += 1;
        }
    }
    out
}

fn brace_class_ranges(text: &str) -> Vec<(String, u32, u32)> {
    let lines: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let s = strip_mods(lines[i].trim_start());
        if let Some(rest) = s.strip_prefix("class") {
            if rest.starts_with(|c: char| c.is_whitespace()) {
                if let Some((name, _)) = read_ident(rest) {
                    let (mut depth, mut seen, mut j) = (0i32, false, i);
                    while j < lines.len() {
                        for c in lines[j].chars() {
                            if c == '{' {
                                depth += 1;
                                seen = true;
                            } else if c == '}' {
                                depth -= 1;
                            }
                        }
                        if seen && depth <= 0 {
                            break;
                        }
                        j += 1;
                    }
                    out.push((name.to_string(), i as u32, j as u32));
                    i = j + 1;
                    continue;
                }
            }
        }
        i += 1;
    }
    out
}

fn strip_mods(mut s: &str) -> &str {
    loop {
        let mut advanced = false;
        for m in MODS {
            if let Some(rest) = s.strip_prefix(m) {
                if rest.starts_with(|c: char| c.is_whitespace()) {
                    s = rest.trim_start();
                    advanced = true;
                    break;
                }
            }
        }
        if !advanced {
            return s;
        }
    }
}

fn read_ident(s: &str) -> Option<(&str, &str)> {
    let s = s.trim_start();
    if !s.starts_with(|c: char| c.is_alphabetic() || c == '_') {
        return None;
    }
    let end = s
        .char_indices()
        .find(|(_, c)| !(c.is_alphanumeric() || *c == '_'))
        .map(|(i, _)| i)
        .unwrap_or(s.len());
    Some((&s[..end], &s[end..]))
}

fn read_type_name(s: &str) -> String {
    s.chars().take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '.').collect()
}

pub fn members_of(files: &[&FileAnalysis], class: &str) -> Vec<Member> {
    let map: HashMap<&str, &ClassData> =
        files.iter().flat_map(|f| f.classes.iter()).map(|c| (c.name.as_str(), c)).collect();
    let mut out = Vec::new();
    let mut seen = HashSet::new();

    fn walk<'a>(
        name: &str,
        map: &HashMap<&str, &'a ClassData>,
        out: &mut Vec<Member>,
        seen: &mut HashSet<String>,
        visited: &mut HashSet<String>,
    ) {
        if !visited.insert(name.to_string()) {
            return;
        }
        let Some(cd) = map.get(name) else { return };
        for m in &cd.members {
            if seen.insert(m.name.clone()) {
                out.push(m.clone());
            }
        }
        for mx in &cd.mixins {
            walk(mx, map, out, seen, visited);
        }
        if let Some(p) = &cd.parent {
            walk(p, map, out, seen, visited);
        }
    }
    walk(class, &map, &mut out, &mut seen, &mut HashSet::new());
    out
}

pub fn is_class(files: &[&FileAnalysis], name: &str) -> bool {
    files.iter().any(|f| f.classes.iter().any(|c| c.name == name))
}

pub fn effective_type(files: &[&FileAnalysis], ty: &str) -> String {
    fn go(files: &[&FileAnalysis], ty: &str, depth: u8) -> String {
        if depth > 6 || ty.is_empty() || is_class(files, ty) {
            return ty.to_string();
        }

        for f in files {
            if let Some(target) = f.alias_targets.get(ty) {
                return go(files, target, depth + 1);
            }
        }
        for f in files {
            if let Some(r) = f.functions.get(ty) {
                return go(files, r, depth + 1);
            }
        }
        ty.to_string()
    }
    go(files, ty, 0)
}

pub fn find_var<'a>(files: &[&'a FileAnalysis], current: &'a FileAnalysis, name: &str) -> Option<&'a VarInfo> {
    if let Some(v) = current.vars.get(name) {
        return Some(v);
    }
    files.iter().find_map(|f| f.vars.get(name).filter(|v| v.global))
}

pub fn self_class(current: &FileAnalysis, line: u32) -> Option<&str> {
    current
        .class_ranges
        .iter()
        .find(|(_, s, e)| line >= *s && line <= *e)
        .map(|(n, _, _)| n.as_str())
}

pub fn type_names(files: &[&FileAnalysis]) -> Vec<(String, &'static str)> {
    let mut v = Vec::new();
    for f in files {
        v.extend(f.classes.iter().map(|c| (c.name.clone(), "class")));
        v.extend(f.interfaces.iter().map(|i| (i.clone(), "interface")));
        v.extend(f.aliases.iter().map(|a| (a.clone(), "type alias")));
    }
    v
}
