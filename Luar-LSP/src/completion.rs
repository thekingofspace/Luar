use crate::infer::{Access, Analysis, Binding, ClassInfo, EnumInfo};
use crate::project::Project;
use crate::resolve::{Resolved, TypeEnv};
use crate::type_syntax::{TypeExpr, BASIC_TYPES};
use crate::types::Type;
use std::path::Path;

pub const KIND_METHOD: i64 = 2;
pub const KIND_FUNCTION: i64 = 3;
pub const KIND_FIELD: i64 = 5;
pub const KIND_VARIABLE: i64 = 6;
pub const KIND_CLASS: i64 = 7;
pub const KIND_INTERFACE: i64 = 8;
pub const KIND_MODULE: i64 = 9;
pub const KIND_PROPERTY: i64 = 10;
pub const KIND_ENUM: i64 = 13;
pub const KIND_KEYWORD: i64 = 14;
pub const KIND_FILE: i64 = 17;
pub const KIND_FOLDER: i64 = 19;
pub const KIND_ENUM_MEMBER: i64 = 20;
pub const KIND_CONSTANT: i64 = 21;
pub const KIND_TYPE_PARAMETER: i64 = 25;

pub const KEYWORDS: [&str; 40] = [
    "local", "const", "pub", "export", "function", "class", "interface", "enum", "type", "if",
    "then", "elseif", "else", "end", "for", "while", "do", "in", "break", "return", "switch",
    "case", "default", "and", "or", "not", "self", "super", "true", "false", "nil", "extends",
    "mixin", "implements", "constructor", "buff", "freebuff", "static", "final", "abstract",
];

#[derive(Debug, Clone, PartialEq)]
pub struct AutoImport {
    pub line0: u32,
    pub new_text: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExtraEdit {
    pub line0: u32,
    pub start_col: u32,
    pub end_col: u32,
    pub new_text: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub label: String,
    pub kind: i64,
    pub detail: String,
    pub insert_text: Option<String>,
    pub is_snippet: bool,
    pub sort_text: Option<String>,
    pub auto_import: Option<AutoImport>,
    pub extra_edit: Option<ExtraEdit>,
}

impl Item {
    fn plain(label: impl Into<String>, kind: i64, detail: impl Into<String>) -> Item {
        Item {
            label: label.into(),
            kind,
            detail: detail.into(),
            insert_text: None,
            is_snippet: false,
            sort_text: None,
            auto_import: None,
            extra_edit: None,
        }
    }

    fn snippet(label: impl Into<String>, kind: i64, detail: impl Into<String>) -> Item {
        let label = label.into();
        let insert = format!("{label}($1)");
        Item {
            label,
            kind,
            detail: detail.into(),
            insert_text: Some(insert),
            is_snippet: true,
            sort_text: None,
            auto_import: None,
            extra_edit: None,
        }
    }

    fn prioritized(mut self) -> Item {
        self.sort_text = Some(format!("0_{}", self.label));
        self
    }
}

pub struct FileView<'a> {
    pub project: &'a Project,
    pub path: &'a Path,
    pub analysis: &'a Analysis,
    pub env: &'a TypeEnv,
    pub annotations: &'a crate::annotations::AnnotationSet,
}

impl<'a> FileView<'a> {
    pub fn from_project(project: &'a Project, path: &'a Path) -> Option<FileView<'a>> {
        let info = project.file(path)?;
        Some(FileView {
            project,
            path,
            analysis: &info.analysis,
            env: &info.env,
            annotations: &info.annotations,
        })
    }

    pub fn find_class(&self, name: &str) -> Option<&ClassInfo> {
        if let Some(c) = self.analysis.classes.get(name) {
            return Some(c);
        }
        if let Some(c) = self.project.luard_classes.get(name) {
            return Some(c);
        }
        if let Some((c, _)) = self.project.pub_classes.get(name) {
            return Some(c);
        }
        self.project
            .files
            .values()
            .find_map(|m| m.analysis.classes.get(name))
    }

    pub fn find_enum(&self, name: &str) -> Option<&EnumInfo> {
        if let Some(e) = self.analysis.enums.get(name) {
            return Some(e);
        }
        if let Some(e) = self.project.luard_enums.get(name) {
            return Some(e);
        }
        if let Some((e, _)) = self.project.pub_enums.get(name) {
            return Some(e);
        }
        self.project
            .files
            .values()
            .find_map(|m| m.analysis.enums.get(name))
    }

    pub fn binding_at(&self, name: &str, line: u32) -> Option<&Binding> {
        self.analysis
            .bindings
            .iter()
            .filter(|b| b.name == name && b.line.map(|l| l <= line).unwrap_or(true))
            .next_back()
            .or_else(|| self.analysis.binding(name))
    }

    pub fn type_of_name(&self, name: &str, line: u32) -> Option<Type> {
        if let Some(b) = self.binding_at(name, line) {
            return Some(b.ty.clone());
        }
        if let Some((_, _, t)) = self
            .analysis
            .param_hints
            .iter()
            .filter(|(l, n, _)| n == name && *l <= line)
            .max_by_key(|(l, _, _)| *l)
        {
            return Some(t.clone());
        }
        if let Some((_, t)) = self
            .project
            .luard_globals
            .iter()
            .find(|(n, _)| n == name)
        {
            return Some(t.clone());
        }
        if let Some((_, t, _)) = self
            .project
            .pub_globals
            .iter()
            .find(|(n, _, p)| n == name && p != self.path)
        {
            return Some(t.clone());
        }
        crate::builtins::global_env().get(name).cloned()
    }

    fn class_chain(&self, name: &str) -> Vec<&ClassInfo> {
        let mut chain = Vec::new();
        let mut current = Some(name.to_string());
        let mut guard = 0;
        while let Some(c) = current {
            guard += 1;
            if guard > 64 {
                break;
            }
            let Some(info) = self.find_class(&c) else {
                break;
            };
            chain.push(info);
            current = info.parent.clone();
        }
        chain
    }

    pub fn expand_named_table(&self, tt: &crate::types::TableType) -> Option<crate::types::TableType> {
        if !tt.fields.is_empty() || tt.array.is_some() {
            return None;
        }
        let name = tt.name.as_ref()?;
        if name == "table" {
            return None;
        }
        match self.env.value_type(&TypeExpr::named(name)) {
            Type::Table(out) if !out.fields.is_empty() || out.array.is_some() => Some(out),
            _ => None,
        }
    }

    pub fn find_interface(&self, name: &str) -> Option<&Vec<String>> {
        if let Some(m) = self.analysis.interfaces.get(name) {
            return Some(m);
        }
        self.project
            .files
            .values()
            .find_map(|f| f.analysis.interfaces.get(name))
    }

    pub fn member_type(&self, ty: &Type, member: &str) -> Type {
        match ty {
            Type::Instance(c) => {
                for info in self.class_chain(c) {
                    if let Some(g) = info.getters.iter().find(|g| g.name == member) {
                        return g.ty.clone();
                    }
                }
                for info in self.class_chain(c) {
                    if let Some(f) = info
                        .fields
                        .iter()
                        .find(|f| f.name == member && !f.is_static)
                    {
                        return f.ty.clone();
                    }
                }
                for info in self.class_chain(c) {
                    if let Some(m) = info.methods.iter().find(|m| m.name == member) {
                        return Type::Function(Some(Box::new(m.sig.clone())));
                    }
                    for mixin in info.mixins.iter().rev() {
                        if let Some(mi) = self.find_class(mixin) {
                            if let Some(m) = mi.methods.iter().find(|m| m.name == member) {
                                return Type::Function(Some(Box::new(m.sig.clone())));
                            }
                        }
                    }
                }
                Type::Unknown
            }
            Type::Class(c) => {
                for info in self.class_chain(c) {
                    if let Some(f) = info
                        .fields
                        .iter()
                        .find(|f| f.name == member && f.is_static)
                    {
                        return f.ty.clone();
                    }
                    if let Some(m) = info.methods.iter().find(|m| m.name == member) {
                        return Type::Function(Some(Box::new(m.sig.clone())));
                    }
                }
                Type::Unknown
            }
            Type::Enum(e) => {
                let known = self
                    .find_enum(e)
                    .map(|info| info.variants.iter().any(|(n, _)| n == member))
                    .unwrap_or(false);
                if known {
                    Type::EnumValue(e.clone())
                } else {
                    Type::Unknown
                }
            }
            Type::Table(tt) => {
                if let Some((_, t)) = tt.fields.iter().find(|(n, _)| n == member) {
                    return t.clone();
                }
                if let Some(expanded) = self.expand_named_table(tt) {
                    return self.member_type(&Type::Table(expanded), member);
                }
                Type::Unknown
            }
            Type::Interface(_) => Type::Unknown,
            Type::Union(parts) => {
                let collected: Vec<Type> = parts
                    .iter()
                    .map(|p| self.member_type(p, member))
                    .filter(|t| *t != Type::Unknown)
                    .collect();
                if collected.is_empty() {
                    Type::Unknown
                } else {
                    Type::union_of(collected)
                }
            }
            _ => Type::Unknown,
        }
    }

    pub fn call_result(&self, ty: &Type) -> Type {
        match ty {
            Type::Function(Some(ft)) => ft.returns.first().cloned().unwrap_or(Type::Nil),
            Type::Class(c) => Type::Instance(c.clone()),
            Type::Table(tt) => {
                let fields = self
                    .expand_named_table(tt)
                    .map(|e| e.fields)
                    .unwrap_or_else(|| tt.fields.clone());
                match fields.iter().find(|(n, _)| n == "__call") {
                    Some((_, Type::Function(Some(ft)))) => {
                        ft.returns.first().cloned().unwrap_or(Type::Nil)
                    }
                    _ => Type::Unknown,
                }
            }
            _ => Type::Unknown,
        }
    }

    pub fn resolve_chain(&self, segments: &[ChainSeg], line: u32) -> Option<Type> {
        self.resolve_chain_in(segments, line, None)
    }

    pub fn resolve_chain_in(
        &self,
        segments: &[ChainSeg],
        line: u32,
        self_class: Option<&str>,
    ) -> Option<Type> {
        let first = segments.first()?;
        let mut ty = match (first.name.as_str(), self_class) {
            ("self", Some(class)) => Type::Instance(class.to_string()),
            ("super", Some(class)) => {
                let parent = self.find_class(class)?.parent.clone()?;
                Type::Instance(parent)
            }
            _ => self.type_of_name(&first.name, line)?,
        };
        if first.called {
            ty = self.call_result(&ty);
        }
        for seg in &segments[1..] {
            ty = self.member_type(&ty, &seg.name);
            if seg.called {
                ty = self.call_result(&ty);
            }
            if ty == Type::Unknown {
                return Some(Type::Unknown);
            }
        }
        Some(ty)
    }

    fn member_visible(&self, access: Access, declaring: &str, viewer: Option<&str>) -> bool {
        match access {
            Access::Public => true,
            Access::Protected => viewer
                .map(|v| self.class_chain(v).iter().any(|c| c.name == declaring))
                .unwrap_or(false),
            Access::Private => viewer == Some(declaring),
        }
    }

    pub fn members_of(&self, ty: &Type, colon: bool, viewer: Option<&str>) -> Vec<Item> {
        let mut items = Vec::new();
        match ty {
            Type::Instance(c) => {
                let mut seen = std::collections::HashSet::new();
                for info in self.class_chain(c) {
                    if colon {
                        for m in info.methods.iter().filter(|m| {
                            !m.is_static && self.member_visible(m.access, &info.name, viewer)
                        }) {
                            if seen.insert(m.name.clone()) {
                                items.push(Item::snippet(
                                    &m.name,
                                    KIND_METHOD,
                                    m.sig.to_string(),
                                ));
                            }
                        }
                        for mixin in info.mixins.iter().rev() {
                            if let Some(mi) = self.find_class(mixin) {
                                for m in mi.methods.iter().filter(|m| {
                                    !m.is_static
                                        && self.member_visible(m.access, &mi.name, viewer)
                                }) {
                                    if seen.insert(m.name.clone()) {
                                        items.push(Item::snippet(
                                            &m.name,
                                            KIND_METHOD,
                                            m.sig.to_string(),
                                        ));
                                    }
                                }
                            }
                        }
                    } else {
                        for g in info.getters.iter().filter(|g| {
                            self.member_visible(g.access, &info.name, viewer)
                        }) {
                            if seen.insert(g.name.clone()) {
                                items.push(Item::plain(&g.name, KIND_PROPERTY, g.ty.to_string()));
                            }
                        }
                        for (s, access) in info.setters.iter().filter(|(_, a)| {
                            self.member_visible(*a, &info.name, viewer)
                        }) {
                            let _ = access;
                            if seen.insert(s.clone()) {
                                items.push(Item::plain(s, KIND_PROPERTY, "setter"));
                            }
                        }
                        for f in info.fields.iter().filter(|f| {
                            !f.is_static && self.member_visible(f.access, &info.name, viewer)
                        }) {
                            if seen.insert(f.name.clone()) {
                                items.push(Item::plain(&f.name, KIND_FIELD, f.ty.to_string()));
                            }
                        }
                        for mixin in info.mixins.iter().rev() {
                            if let Some(mi) = self.find_class(mixin) {
                                for f in mi.fields.iter().filter(|f| {
                                    !f.is_static
                                        && self.member_visible(f.access, &mi.name, viewer)
                                }) {
                                    if seen.insert(f.name.clone()) {
                                        items.push(Item::plain(
                                            &f.name,
                                            KIND_FIELD,
                                            f.ty.to_string(),
                                        ));
                                    }
                                }
                            }
                        }
                    }
                }
            }
            Type::Class(c) => {
                let mut seen = std::collections::HashSet::new();
                for info in self.class_chain(c) {
                    for f in info.fields.iter().filter(|f| {
                        f.is_static && self.member_visible(f.access, &info.name, viewer)
                    }) {
                        if seen.insert(f.name.clone()) {
                            items.push(
                                Item::plain(&f.name, KIND_FIELD, format!("static {}", f.ty))
                                    .prioritized(),
                            );
                        }
                    }
                    for m in &info.methods {
                        if self.member_visible(m.access, &info.name, viewer)
                            && seen.insert(m.name.clone())
                        {
                            items.push(Item::snippet(&m.name, KIND_METHOD, m.sig.to_string()));
                        }
                    }
                    for f in info.fields.iter().filter(|f| {
                        !f.is_static && self.member_visible(f.access, &info.name, viewer)
                    }) {
                        if seen.insert(f.name.clone()) {
                            items.push(Item::plain(&f.name, KIND_FIELD, f.ty.to_string()));
                        }
                    }
                    for g in info.getters.iter().filter(|g| {
                        self.member_visible(g.access, &info.name, viewer)
                    }) {
                        if seen.insert(g.name.clone()) {
                            items.push(Item::plain(&g.name, KIND_PROPERTY, g.ty.to_string()));
                        }
                    }
                }
                let _ = colon;
            }
            Type::Interface(n) => {
                if let Some(members) = self.find_interface(n) {
                    for m in members {
                        items.push(Item::plain(m, KIND_PROPERTY, "interface member"));
                    }
                }
            }
            Type::Enum(e) => {
                if let Some(info) = self.find_enum(e) {
                    for (v, t) in &info.variants {
                        items.push(Item::plain(v, KIND_ENUM_MEMBER, t.to_string()));
                    }
                }
            }
            Type::Table(tt) => {
                let expanded = self.expand_named_table(tt);
                let fields = expanded.as_ref().map(|e| &e.fields).unwrap_or(&tt.fields);
                for (name, t) in fields {
                    if !is_plain_ident(name) {
                        let form = format!("[\"{name}\"]");
                        let mut item = Item::plain(&form, KIND_FIELD, t.to_string());
                        item.insert_text = Some(form);
                        item.sort_text = Some(format!("z_{name}"));
                        items.push(item);
                        continue;
                    }
                    match t {
                        Type::Function(Some(sig)) => {
                            let detail = if colon
                                && sig.params.first().map(|p| p.name.as_str()) == Some("self")
                            {
                                let mut shown = (**sig).clone();
                                shown.params.remove(0);
                                shown.to_string()
                            } else {
                                sig.to_string()
                            };
                            items.push(Item::snippet(name, KIND_FUNCTION, detail));
                        }
                        other => items.push(Item::plain(name, KIND_FIELD, other.to_string())),
                    }
                }
            }
            Type::Union(parts) => {
                let mut seen = std::collections::HashSet::new();
                for p in parts {
                    for item in self.members_of(p, colon, viewer) {
                        if seen.insert(item.label.clone()) {
                            items.push(item);
                        }
                    }
                }
            }
            _ => {}
        }
        items
    }

    pub fn literal_strings(&self, texpr: &TypeExpr) -> Vec<String> {
        let mut out = Vec::new();
        self.collect_literals(texpr, &mut out, 0);
        out.sort();
        out.dedup();
        out
    }

    fn collect_literals(&self, texpr: &TypeExpr, out: &mut Vec<String>, depth: usize) {
        if depth > 16 {
            return;
        }
        match texpr {
            TypeExpr::StringLit(s) => out.push(s.clone()),
            TypeExpr::Union(parts) | TypeExpr::Intersection(parts) => {
                for p in parts {
                    self.collect_literals(p, out, depth + 1);
                }
            }
            TypeExpr::Optional(inner) => self.collect_literals(inner, out, depth + 1),
            TypeExpr::Named(_) => match self.env.resolve(texpr) {
                Resolved::StringLiteral(s) => out.push(s),
                Resolved::Structural(inner) if inner != *texpr => {
                    self.collect_literals(&inner, out, depth + 1)
                }
                _ => {}
            },
            _ => {}
        }
    }

    pub fn annotation_for(&self, name: &str, before_line: u32) -> Option<&TypeExpr> {
        self.annotations
            .vars
            .iter()
            .filter(|((n, l), _)| n == name && *l <= before_line)
            .max_by_key(|((_, l), _)| *l)
            .map(|(_, t)| t)
    }
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

pub fn word_at(line: &str, col: usize) -> Option<(String, usize, usize)> {
    let chars: Vec<char> = line.chars().collect();
    if chars.is_empty() {
        return None;
    }
    let mut start = col.min(chars.len());
    while start > 0 && is_word_char(chars[start - 1]) {
        start -= 1;
    }
    let mut end = col.min(chars.len());
    while end < chars.len() && is_word_char(chars[end]) {
        end += 1;
    }
    if start == end {
        return None;
    }
    Some((chars[start..end].iter().collect(), start, end))
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChainSeg {
    pub name: String,
    pub called: bool,
}

pub struct ChainAt {
    pub segments: Vec<ChainSeg>,
    pub separator: char,
    pub partial: String,
    pub cast_base: Option<String>,
}

pub fn chain_before(line: &str, col: usize) -> Option<ChainAt> {
    let chars: Vec<char> = line.chars().collect();
    let mut i = col.min(chars.len());
    let mut partial_start = i;
    while partial_start > 0 && is_word_char(chars[partial_start - 1]) {
        partial_start -= 1;
    }
    let partial: String = chars[partial_start..i].iter().collect();
    i = partial_start;
    if i == 0 {
        return None;
    }
    let sep = chars[i - 1];
    if sep != '.' && sep != ':' {
        return None;
    }
    if i >= 2 && (chars[i - 2] == ':' || chars[i - 2] == '.') {
        return None;
    }
    i -= 1;
    let mut segments: Vec<ChainSeg> = Vec::new();
    let mut cast_base: Option<String> = None;
    loop {
        let mut called = false;
        let mut group_start: Option<usize> = None;
        if i > 0 && chars[i - 1] == ']' {
            let close = i - 1;
            if close == 0 {
                return None;
            }
            let q = chars[close - 1];
            if q != '"' && q != '\'' {
                return None;
            }
            let str_end = close - 1;
            let mut k = str_end;
            loop {
                if k == 0 {
                    return None;
                }
                k -= 1;
                if chars[k] == q {
                    break;
                }
            }
            if k == 0 || chars[k - 1] != '[' {
                return None;
            }
            let key: String = chars[k + 1..str_end].iter().collect();
            segments.push(ChainSeg { name: key, called: false });
            i = k - 1;
            if i > 0 && (chars[i - 1] == '.' || chars[i - 1] == ':') {
                return None;
            }
            if i == 0 {
                break;
            }
            continue;
        }
        if i > 0 && chars[i - 1] == ')' {
            called = true;
            let close = i - 1;
            let mut depth = 1;
            i -= 1;
            while i > 0 && depth > 0 {
                i -= 1;
                match chars[i] {
                    ')' => depth += 1,
                    '(' => depth -= 1,
                    _ => {}
                }
            }
            if depth > 0 {
                return None;
            }
            group_start = Some(i);
            let _ = close;
        }
        let mut start = i;
        while start > 0 && is_word_char(chars[start - 1]) {
            start -= 1;
        }
        if start == i {
            if let Some(open) = group_start {
                if segments.is_empty() || !segments.is_empty() {
                    let inner: String = chars[open + 1..].iter().collect();
                    let inner = match inner.rfind(')') {
                        Some(_) => {
                            let group_chars: Vec<char> = chars[open + 1..].to_vec();
                            let mut depth = 0i32;
                            let mut end = group_chars.len();
                            for (gi, c) in group_chars.iter().enumerate() {
                                match c {
                                    '(' => depth += 1,
                                    ')' => {
                                        if depth == 0 {
                                            end = gi;
                                            break;
                                        }
                                        depth -= 1;
                                    }
                                    _ => {}
                                }
                            }
                            group_chars[..end].iter().collect::<String>()
                        }
                        None => inner,
                    };
                    if let Some(pos) = inner.rfind("::") {
                        cast_base = Some(inner[pos + 2..].trim().to_string());
                        break;
                    }
                }
            }
            return None;
        }
        segments.push(ChainSeg {
            name: chars[start..i].iter().collect(),
            called,
        });
        i = start;
        if i > 0 && (chars[i - 1] == '.' || chars[i - 1] == ':') {
            if i >= 2 && (chars[i - 2] == ':' || chars[i - 2] == '.') {
                return None;
            }
            i -= 1;
        } else {
            break;
        }
    }
    segments.reverse();
    Some(ChainAt {
        segments,
        separator: sep,
        partial,
        cast_base,
    })
}

pub fn require_partial(line: &str, col: usize) -> Option<String> {
    let upto: String = line.chars().take(col).collect();
    let req = upto.rfind("require")?;
    let after = &upto[req + "require".len()..];
    let after = after.trim_start();
    let after = after.strip_prefix('(').unwrap_or(after);
    let after = after.trim_start();
    let quote = after.chars().next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let body = &after[1..];
    if body.contains(quote) {
        return None;
    }
    Some(body.to_string())
}

fn in_open_string(line: &str, col: usize) -> Option<(char, String)> {
    let chars: Vec<char> = line.chars().take(col).collect();
    let mut open: Option<(char, usize)> = None;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match open {
            None => {
                if c == '"' || c == '\'' {
                    open = Some((c, i));
                } else if c == '-' && chars.get(i + 1) == Some(&'-') {
                    return None;
                }
            }
            Some((q, _)) => {
                if c == '\\' {
                    i += 1;
                } else if c == q {
                    open = None;
                }
            }
        }
        i += 1;
    }
    open.map(|(q, start)| (q, chars[start + 1..].iter().collect()))
}

const DIRECTIVE_KEYWORDS: [&str; 3] = ["disable", "disable-line", "disable-next-line"];

const EDITOR_CHECKS: [&str; 7] = [
    "DuplicateEnumVariant",
    "GenericArity",
    "UnknownClass",
    "RequireCycle",
    "NotNil",
    "FinalOverride",
    "all",
];

fn directive_completion(line: &str, col: usize) -> Option<Vec<Item>> {
    let prefix: String = line.chars().take(col).collect();
    let idx = prefix.rfind("--#")?;
    let after = &prefix[idx + 3..];
    if !after.contains(char::is_whitespace) {
        let items: Vec<Item> = DIRECTIVE_KEYWORDS
            .iter()
            .filter(|k| k.starts_with(after))
            .map(|k| Item::plain(*k, KIND_KEYWORD, "ferrite directive"))
            .collect();
        return Some(items);
    }
    let keyword = after.split_whitespace().next().unwrap_or("");
    if !DIRECTIVE_KEYWORDS.contains(&keyword) {
        return Some(Vec::new());
    }
    let partial = after
        .rsplit(|c: char| c.is_whitespace() || c == ',')
        .next()
        .unwrap_or("");
    let mut names: Vec<&str> = luar::ferrite::CHECKS.to_vec();
    names.extend(EDITOR_CHECKS);
    names.sort();
    let items: Vec<Item> = names
        .iter()
        .filter(|c| partial.is_empty() || c.to_lowercase().starts_with(&partial.to_lowercase()))
        .map(|c| Item::plain(*c, KIND_CONSTANT, "ferrite check"))
        .collect();
    Some(items)
}

pub fn in_comment(line: &str, col: usize) -> bool {
    let chars: Vec<char> = line.chars().take(col).collect();
    let mut quote: Option<char> = None;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match quote {
            None => {
                if c == '"' || c == '\'' || c == '`' {
                    quote = Some(c);
                } else if c == '-' && chars.get(i + 1) == Some(&'-') {
                    return true;
                }
            }
            Some(q) => {
                if c == '\\' {
                    i += 1;
                } else if c == q {
                    quote = None;
                }
            }
        }
        i += 1;
    }
    false
}

pub fn is_type_position(line: &str, col: usize) -> bool {
    let upto: String = line.chars().take(col).collect();
    let trimmed_end =
        upto.trim_end_matches(|c: char| is_word_char(c) || c == '.' || c.is_whitespace());
    if trimmed_end.ends_with("::") {
        return true;
    }
    let head = upto.trim_start();
    if head.starts_with("type ") || head.starts_with("export type ") {
        if let Some(eq) = upto.find('=') {
            return col > eq;
        }
        return false;
    }
    let Some(colon) = find_annotation_colon(&upto) else {
        return false;
    };
    let after_colon = &upto[colon + 1..];
    if after_colon.contains('=') && !after_colon.contains('{') {
        return false;
    }
    let before = upto[..colon].trim_end();
    let before_ok = before.ends_with(|c: char| is_word_char(c))
        || before.ends_with(')')
        || before.ends_with(']');
    if before.is_empty() || !before_ok {
        return false;
    }
    if before.contains('=') {
        return false;
    }
    let decl = head.starts_with("local ")
        || head.starts_with("const ")
        || head.starts_with("pub ")
        || head.starts_with("export ")
        || head.starts_with("public ")
        || head.starts_with("private ")
        || head.starts_with("protected ")
        || head.starts_with("static ");
    if decl {
        return true;
    }
    if function_like_line(&upto) && upto.matches('(').count() > upto.matches(')').count() {
        return true;
    }
    let on_function_line = function_like_line(&upto);
    if on_function_line && (before.ends_with(')') || upto.contains("):")) {
        return true;
    }
    let paren_open = upto.matches('(').count() > upto.matches(')').count();
    if paren_open && on_function_line {
        return true;
    }
    if on_function_line
        && before
            .rfind(')')
            .map(|p| upto[p..].trim_start_matches(')').trim_start().starts_with(':'))
            .unwrap_or(false)
    {
        return true;
    }
    false
}

fn function_like_line(upto: &str) -> bool {
    if line_has_word(upto, "function") {
        return true;
    }
    let mut rest = upto.trim_start();
    loop {
        let word_end = rest
            .find(|c: char| !(c.is_alphanumeric() || c == '_'))
            .unwrap_or(rest.len());
        match &rest[..word_end] {
            "public" | "private" | "protected" | "static" | "abstract" | "final"
            | "override" => rest = rest[word_end..].trim_start(),
            "get" | "set" | "operator" | "constructor" | "destructor" => return true,
            _ => return false,
        }
    }
}

fn find_annotation_colon(upto: &str) -> Option<usize> {
    let bytes = upto.as_bytes();
    let mut i = bytes.len();
    while i > 0 {
        i -= 1;
        if bytes[i] == b':' {
            if i > 0 && bytes[i - 1] == b':' {
                i -= 1;
                continue;
            }
            if i + 1 < bytes.len() && bytes[i + 1] == b':' {
                continue;
            }
            return Some(i);
        }
    }
    None
}

pub fn complete(view: &FileView, text: &str, line0: usize, col0: usize) -> Vec<Item> {
    let line = text.lines().nth(line0).unwrap_or("");
    let cur_line = line0 as u32 + 1;

    if in_comment(line, col0) {
        return directive_completion(line, col0).unwrap_or_default();
    }

    if let Some(partial) = require_partial(line, col0) {
        let typed_at = partial.starts_with('@');
        let mut items: Vec<Item> = view
            .project
            .complete_require(view.path, &partial)
            .into_iter()
            .map(|name| {
                let mut item = Item::plain(&name, KIND_FILE, "module");
                if typed_at && name.starts_with('@') {
                    item.kind = KIND_FOLDER;
                    item.detail = "alias".to_string();
                    item.insert_text = Some(name.trim_start_matches('@').to_string());
                }
                item
            })
            .collect();
        if partial.is_empty() {
            for alias in view.project.aliases.keys() {
                items.push(Item::plain(format!("@{alias}"), KIND_FOLDER, "alias"));
            }
            if view
                .path
                .file_name()
                .map(|n| n == "init.luar")
                .unwrap_or(false)
            {
                items.push(Item::plain("@self", KIND_FOLDER, "this file's directory"));
            }
            items.push(Item::plain("./", KIND_FOLDER, "this file's directory"));
        }
        items.sort_by(|a, b| a.label.cmp(&b.label));
        return items;
    }

    if let Some(items) = bracket_key_completion(view, text, line, line0, col0) {
        return items;
    }

    if let Some((_, body)) = in_open_string(line, col0) {
        let before_string: String = {
            let upto: String = line.chars().take(col0).collect();
            let cut = upto.len() - body.chars().count() - 1;
            upto.chars().take(cut).collect()
        };
        let trimmed = before_string.trim_end();
        let trimmed = trimmed
            .trim_end_matches("==")
            .trim_end_matches("~=")
            .trim_end_matches('=')
            .trim_end();
        if let Some((name, _, _)) = word_at(trimmed, trimmed.chars().count()) {
            if let Some(texpr) = view.annotation_for(&name, cur_line) {
                let texpr = texpr.clone();
                let lits = view.literal_strings(&texpr);
                if !lits.is_empty() {
                    return lits
                        .into_iter()
                        .map(|s| Item::plain(s, KIND_CONSTANT, "literal"))
                        .collect();
                }
            }
        }
        if let Some(items) = call_argument_literals(view, text, &before_string, line0, col0) {
            return items;
        }
        let head = before_string.trim_start();
        if head.starts_with("case") {
            if let Some(subject) = switch_subject(text, line0) {
                if let Some(texpr) = view.annotation_for(&subject, cur_line) {
                    let texpr = texpr.clone();
                    let lits = view.literal_strings(&texpr);
                    if !lits.is_empty() {
                        return lits
                            .into_iter()
                            .map(|s| Item::plain(s, KIND_CONSTANT, "literal"))
                            .collect();
                    }
                }
            }
        }
        return Vec::new();
    }

    let type_pos = is_type_position(line, col0) || in_type_alias_body(text, line0, col0);

    if let Some(chain) = chain_before(line, col0) {
        if type_pos {
            if chain.separator == '.' {
                if let Some(seg) = chain.segments.last() {
                    if !seg.called {
                        if let Some(menv) = view.env.modules.get(&seg.name) {
                            return menv
                                .exported_type_names()
                                .into_iter()
                                .map(|n| Item::plain(n, KIND_CLASS, "exported type"))
                                .collect();
                        }
                    }
                }
                return Vec::new();
            }
        } else {
            let self_class = enclosing_class(text, line0, col0);
            if let Some(ty) = resolve_chain_full(view, &chain, cur_line, self_class.as_deref()) {
                let mut items =
                    view.members_of(&ty, chain.separator == ':', self_class.as_deref());
                if chain.separator == '.' {
                    let dot_col = col0.saturating_sub(chain.partial.chars().count() + 1) as u32;
                    for item in items.iter_mut() {
                        if item.label.starts_with("[\"") {
                            item.extra_edit = Some(ExtraEdit {
                                line0: line0 as u32,
                                start_col: dot_col,
                                end_col: dot_col + 1,
                                new_text: String::new(),
                            });
                        }
                    }
                } else {
                    items.retain(|i| !i.label.starts_with("[\""));
                }
                return items;
            }
            return Vec::new();
        }
    }

    if type_pos {
        let mut items: Vec<Item> = BASIC_TYPES
            .iter()
            .map(|b| Item::plain(*b, KIND_KEYWORD, "basic type"))
            .collect();
        items.push(Item::plain("any", KIND_KEYWORD, "basic type"));
        items.push(Item::plain("void", KIND_KEYWORD, "basic type"));
        items.push(Item::plain("integer", KIND_KEYWORD, "number (documentation only)"));
        items.push(Item::plain("double", KIND_KEYWORD, "number (documentation only)"));
        items.push(Item::plain("float", KIND_KEYWORD, "number (documentation only)"));
        for g in generic_params_in_scope(text, line0) {
            items.push(Item::plain(g, KIND_TYPE_PARAMETER, "generic parameter").prioritized());
        }
        items.push(Item {
            label: "keyof".to_string(),
            kind: KIND_KEYWORD,
            detail: "keyof<T> — union of T's field names".to_string(),
            insert_text: Some("keyof<$1>".to_string()),
            is_snippet: true,
            sort_text: None,
            auto_import: None,
            extra_edit: None,
        });
        items.push(Item {
            label: "ValueOf".to_string(),
            kind: KIND_KEYWORD,
            detail: "ValueOf<T, K> — type of T's field K".to_string(),
            insert_text: Some("ValueOf<$1, $2>".to_string()),
            is_snippet: true,
            sort_text: None,
            auto_import: None,
            extra_edit: None,
        });
        items.push(Item {
            label: "ToBasic".to_string(),
            kind: KIND_KEYWORD,
            detail: "ToBasic<T> — widen a literal to its basic type".to_string(),
            insert_text: Some("ToBasic<$1>".to_string()),
            is_snippet: true,
            sort_text: None,
            auto_import: None,
            extra_edit: None,
        });
        items.push(Item {
            label: "NotNil".to_string(),
            kind: KIND_KEYWORD,
            detail: "NotNil<T> — like T but assigning nil is an error".to_string(),
            insert_text: Some("NotNil<$1>".to_string()),
            is_snippet: true,
            sort_text: None,
            auto_import: None,
            extra_edit: None,
        });
        for name in view.env.type_names() {
            let kind = if view.env.classes.contains(&name) {
                KIND_CLASS
            } else if view.env.enums.contains(&name) {
                KIND_ENUM
            } else if view.env.interfaces.contains(&name) {
                KIND_INTERFACE
            } else {
                KIND_CLASS
            };
            items.push(Item::plain(name, kind, "type"));
        }
        for module in view.env.modules.keys() {
            items.push(Item::plain(module.clone(), KIND_MODULE, "module types"));
        }
        items.sort_by(|a, b| a.label.cmp(&b.label));
        items.dedup_by(|a, b| a.label == b.label);
        return items;
    }

    if let Some(member_items) = class_member_completion(text, line0, col0) {
        return member_items;
    }
    let mut items: Vec<Item> = Vec::new();
    for kw in KEYWORDS {
        if kw == "switch" {
            items.push(Item {
                label: "switch".to_string(),
                kind: KIND_KEYWORD,
                detail: "switch(value) case ... end end".to_string(),
                insert_text: Some("switch($1)".to_string()),
                is_snippet: true,
                sort_text: None,
                auto_import: None,
                extra_edit: None,
            });
        } else if kw == "else" || kw == "elseif" {
            let mut item = Item::plain(kw, KIND_KEYWORD, "keyword");
            item.extra_edit = branch_indent_edit(text, line, line0, col0);
            items.push(item);
        } else {
            items.push(Item::plain(kw, KIND_KEYWORD, "keyword"));
        }
    }
    let mut seen = std::collections::HashSet::new();
    for (name, detail) in params_in_scope(text, line0, col0) {
        if seen.insert(name.clone()) {
            items.push(Item::plain(name, KIND_VARIABLE, detail));
        }
    }
    for b in view.analysis.bindings.iter().rev() {
        if b.line.map(|l| l <= cur_line).unwrap_or(true) && seen.insert(b.name.clone()) {
            let (kind, detail) = item_kind_for(&b.ty);
            match &b.ty {
                Type::Function(Some(_)) => {
                    items.push(Item::snippet(&b.name, kind, detail));
                }
                _ => items.push(Item::plain(&b.name, kind, detail)),
            }
        }
    }
    for (name, ty) in &view.project.luard_globals {
        if seen.insert(name.clone()) {
            let (kind, detail) = item_kind_for(ty);
            match ty {
                Type::Function(Some(_)) => items.push(Item::snippet(name, kind, detail)),
                _ => items.push(Item::plain(name, kind, detail)),
            }
        }
    }
    for (name, ty, src_path) in &view.project.pub_globals {
        if src_path != view.path && seen.insert(name.clone()) {
            let (kind, detail) = item_kind_for(ty);
            match ty {
                Type::Function(Some(_)) => items.push(Item::snippet(name, kind, detail)),
                _ => items.push(Item::plain(name, kind, detail)),
            }
        }
    }
    for (name, ty) in crate::builtins::global_env() {
        if seen.insert(name.clone()) {
            let (kind, detail) = item_kind_for(&ty);
            match ty {
                Type::Function(Some(_)) => items.push(Item::snippet(&name, kind, detail)),
                _ => items.push(Item::plain(&name, kind, detail)),
            }
        }
    }
    items.extend(auto_import_items(view, text, &mut seen));
    items.extend(declared_auto_import_items(view, text, &mut seen));
    items.sort_by(|a, b| a.label.cmp(&b.label));
    items.dedup_by(|a, b| a.label == b.label);
    items
}

fn declared_auto_import_items(
    view: &FileView,
    text: &str,
    seen: &mut std::collections::HashSet<String>,
) -> Vec<Item> {
    if view.project.auto_imports.is_empty() {
        return Vec::new();
    }
    let insert_line = require_insert_line(text);
    let eol = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let needs_leading_eol =
        insert_line >= text.lines().count() && !text.is_empty() && !text.ends_with('\n');
    let mut cluster: Vec<(usize, String)> = Vec::new();
    for (row, line) in text.lines().enumerate() {
        let trimmed = line.trim_start();
        let Some(rest) = trimmed.strip_prefix("local ") else {
            continue;
        };
        let name: String = rest
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        if name.is_empty() || !rest[name.len()..].trim_start().starts_with('=') {
            continue;
        }
        if view.project.auto_imports.iter().any(|(k, _)| *k == name) {
            cluster.push((row, name));
        }
    }
    cluster.sort_by(|a, b| a.1.cmp(&b.1));
    let mut items = Vec::new();
    for (key, expr) in &view.project.auto_imports {
        if seen.contains(key) || view.analysis.binding(key).is_some() {
            continue;
        }
        seen.insert(key.clone());
        let (line0, prefix) = if cluster.is_empty() {
            (
                insert_line,
                if needs_leading_eol { eol } else { "" },
            )
        } else {
            let row = match cluster.iter().find(|(_, k)| k.as_str() > key.as_str()) {
                Some((row, _)) => *row,
                None => cluster.iter().map(|(r, _)| *r).max().unwrap_or(0) + 1,
            };
            (row, "")
        };
        let new_text = format!("{prefix}local {key} = {expr}{eol}");
        items.push(Item {
            label: key.clone(),
            kind: KIND_VARIABLE,
            detail: format!("auto-import — {expr}"),
            insert_text: None,
            is_snippet: false,
            sort_text: Some(format!("z_{key}")),
            auto_import: Some(AutoImport {
                line0: line0 as u32,
                new_text,
            }),
            extra_edit: None,
        });
    }
    items
}

fn auto_import_items(
    view: &FileView,
    text: &str,
    seen: &mut std::collections::HashSet<String>,
) -> Vec<Item> {
    let project = view.project;
    let already: &[std::path::PathBuf] = project
        .file(view.path)
        .map(|info| info.requires.as_slice())
        .unwrap_or(&[]);
    let insert_line = require_insert_line(text);
    let eol = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let needs_leading_eol =
        insert_line >= text.lines().count() && !text.is_empty() && !text.ends_with('\n');
    let mut items = Vec::new();
    for path in project.files.keys() {
        if path == view.path || already.contains(path) {
            continue;
        }
        let Some(name) = module_display_name(path) else {
            continue;
        };
        if seen.contains(&name) {
            continue;
        }
        let Some(req) = best_require_path(project, view.path, path) else {
            continue;
        };
        seen.insert(name.clone());
        let prefix = if needs_leading_eol { eol } else { "" };
        let new_text = format!("{prefix}local {name} = require(\"{req}\"){eol}");
        items.push(Item {
            label: name.clone(),
            kind: KIND_MODULE,
            detail: format!("auto-import — require(\"{req}\")"),
            insert_text: None,
            is_snippet: false,
            sort_text: Some(format!("z_{name}")),
            auto_import: Some(AutoImport {
                line0: insert_line as u32,
                new_text,
            }),
            extra_edit: None,
        });
    }
    items
}

fn module_display_name(path: &Path) -> Option<String> {
    let stem = path.file_stem()?.to_string_lossy();
    let name = if stem == "init" {
        path.parent()?.file_name()?.to_string_lossy().into_owned()
    } else {
        stem.into_owned()
    };
    let mut chars = name.chars();
    let first = chars.next()?;
    if !(first.is_alphabetic() || first == '_') {
        return None;
    }
    if !chars.all(|c| c.is_alphanumeric() || c == '_') {
        return None;
    }
    Some(name)
}

fn is_require_call(e: &luar::ast::Expr) -> bool {
    match e {
        luar::ast::Expr::Call { callee, args } => {
            matches!(callee.as_ref(), luar::ast::Expr::Name(f) if f == "require")
                && matches!(args.first(), Some(luar::ast::Expr::Str(_)))
        }
        _ => false,
    }
}

fn require_insert_line(text: &str) -> usize {
    let (program, _, _) = crate::parse_source_repaired_with_errors(text);
    let mut last = None;
    for s in &program {
        match s {
            luar::ast::Stmt::Declare { inits, line, .. } if inits.iter().any(is_require_call) => {
                last = Some(*line as usize);
            }
            luar::ast::Stmt::Expr(e, line) if is_require_call(e) => {
                last = Some(*line as usize);
            }
            _ => {}
        }
    }
    last.unwrap_or(0)
}

fn best_require_path(project: &Project, from: &Path, target: &Path) -> Option<String> {
    let target_mod = if target.file_stem().map(|s| s == "init").unwrap_or(false) {
        target.parent()?.to_path_buf()
    } else {
        target.with_extension("")
    };
    let mut alias_best: Option<String> = None;
    for (alias, t) in &project.aliases {
        let cleaned = t.trim_start_matches("./").trim_end_matches('/');
        let base = if cleaned.is_empty() {
            project.root.clone()
        } else {
            project.root.join(cleaned)
        };
        let candidate = if base == target_mod {
            Some(format!("@{alias}"))
        } else if let Ok(rel) = target_mod.strip_prefix(&base) {
            Some(format!("@{alias}/{}", slashed(rel)))
        } else {
            None
        };
        if let Some(c) = candidate {
            if alias_best.as_ref().map(|b| c.len() < b.len()).unwrap_or(true) {
                alias_best = Some(c);
            }
        }
    }
    if alias_best.is_some() {
        return alias_best;
    }
    let from_dir = from.parent()?;
    let effective = if from.file_name().map(|n| n == "init.luar").unwrap_or(false) {
        from_dir.parent()?
    } else {
        from_dir
    };
    let mut base = effective.to_path_buf();
    let mut ups = 0usize;
    loop {
        if let Ok(rel) = target_mod.strip_prefix(&base) {
            let rel_s = slashed(rel);
            if rel_s.is_empty() {
                return None;
            }
            let dots = ".".repeat(ups + 1);
            return Some(format!("{dots}/{rel_s}"));
        }
        base = base.parent()?.to_path_buf();
        ups += 1;
    }
}

fn slashed(p: &Path) -> String {
    p.components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

pub const CLASS_MEMBER_KEYWORDS: [&str; 13] = [
    "public",
    "private",
    "protected",
    "static",
    "abstract",
    "final",
    "override",
    "function",
    "constructor",
    "destructor",
    "operator",
    "get",
    "set",
];

const MEMBER_MODIFIERS: [&str; 7] = [
    "public",
    "private",
    "protected",
    "static",
    "abstract",
    "final",
    "override",
];

struct BlockFrame {
    is_function: bool,
    vars: Vec<(String, &'static str)>,
}

fn strip_to_code(text: &str) -> String {
    enum Mode {
        Code,
        Line,
        Block(usize),
        Quote(char),
        Long(usize),
        Interp,
    }
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    let mut out = String::with_capacity(n);
    let mut mode = Mode::Code;
    let mut i = 0;
    let long_open_at = |pos: usize| -> Option<usize> {
        if chars.get(pos) != Some(&'[') {
            return None;
        }
        let mut j = pos + 1;
        while chars.get(j) == Some(&'=') {
            j += 1;
        }
        if chars.get(j) == Some(&'[') {
            Some(j - pos - 1)
        } else {
            None
        }
    };
    while i < n {
        let c = chars[i];
        if c == '\n' {
            out.push('\n');
            if matches!(mode, Mode::Line) {
                mode = Mode::Code;
            }
            i += 1;
            continue;
        }
        match mode {
            Mode::Code => {
                if c == '-' && chars.get(i + 1) == Some(&'-') {
                    if let Some(lvl) = long_open_at(i + 2) {
                        mode = Mode::Block(lvl);
                        for _ in 0..(4 + lvl) {
                            out.push(' ');
                        }
                        i += 4 + lvl;
                    } else {
                        mode = Mode::Line;
                        out.push(' ');
                        i += 1;
                    }
                    continue;
                }
                if c == '"' || c == '\'' {
                    mode = Mode::Quote(c);
                    out.push(' ');
                    i += 1;
                    continue;
                }
                if c == '`' {
                    mode = Mode::Interp;
                    out.push(' ');
                    i += 1;
                    continue;
                }
                if let Some(lvl) = long_open_at(i) {
                    mode = Mode::Long(lvl);
                    for _ in 0..(2 + lvl) {
                        out.push(' ');
                    }
                    i += 2 + lvl;
                    continue;
                }
                out.push(c);
                i += 1;
            }
            Mode::Line => {
                out.push(' ');
                i += 1;
            }
            Mode::Quote(q) => {
                out.push(' ');
                if c == '\\' {
                    if i + 1 < n && chars[i + 1] != '\n' {
                        out.push(' ');
                        i += 2;
                        continue;
                    }
                } else if c == q {
                    mode = Mode::Code;
                }
                i += 1;
            }
            Mode::Interp => {
                out.push(' ');
                if c == '\\' {
                    if i + 1 < n && chars[i + 1] != '\n' {
                        out.push(' ');
                        i += 2;
                        continue;
                    }
                } else if c == '`' {
                    mode = Mode::Code;
                }
                i += 1;
            }
            Mode::Block(lvl) | Mode::Long(lvl) => {
                if c == ']' {
                    let mut j = i + 1;
                    let mut eq = 0;
                    while chars.get(j) == Some(&'=') {
                        eq += 1;
                        j += 1;
                    }
                    if eq == lvl && chars.get(j) == Some(&']') {
                        for _ in 0..(j + 1 - i) {
                            out.push(' ');
                        }
                        i = j + 1;
                        mode = Mode::Code;
                        continue;
                    }
                }
                out.push(' ');
                i += 1;
            }
        }
    }
    out
}

#[derive(Default)]
struct ScanCarry {
    pending_loop: bool,
}

fn block_stack_at(text: &str, line0: usize, col0: usize) -> Vec<BlockFrame> {
    let cleaned_all = strip_to_code(text);
    let mut stack: Vec<BlockFrame> = Vec::new();
    let mut carry = ScanCarry::default();
    for (i, raw) in cleaned_all.lines().enumerate().take(line0 + 1) {
        let upto: String = if i == line0 {
            raw.chars().take(col0).collect()
        } else {
            raw.to_string()
        };
        scan_line_blocks(&upto, &mut stack, &mut carry);
    }
    stack
}

pub fn params_in_scope(text: &str, line0: usize, col0: usize) -> Vec<(String, &'static str)> {
    let stack = block_stack_at(text, line0, col0);
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for frame in stack.iter().rev() {
        for (name, kind) in &frame.vars {
            if seen.insert(name.clone()) {
                out.push((name.clone(), *kind));
            }
        }
    }
    out
}

pub(crate) fn inside_function_block(text: &str, line0: usize, col0: usize) -> bool {
    block_stack_at(text, line0, col0)
        .iter()
        .any(|f| f.is_function)
}

fn scan_line_blocks(line: &str, stack: &mut Vec<BlockFrame>, carry: &mut ScanCarry) {
    let mut prev = String::new();
    let mut saw_loop_word = false;
    let mut loop_awaiting_do = carry.pending_loop;
    let mut chars = line.char_indices().peekable();
    while let Some((idx, c)) = chars.next() {
        if !(c.is_alphanumeric() || c == '_') {
            continue;
        }
        let start = idx;
        let mut end = idx + c.len_utf8();
        while let Some((j, cc)) = chars.peek().copied() {
            if cc.is_alphanumeric() || cc == '_' {
                end = j + cc.len_utf8();
                chars.next();
            } else {
                break;
            }
        }
        let word = &line[start..end];
        match word {
            "if" | "while" | "switch" | "case" | "default" => {
                stack.push(BlockFrame { is_function: false, vars: Vec::new() })
            }
            "for" => stack.push(BlockFrame {
                is_function: false,
                vars: loop_vars_after(&line[end..]),
            }),
            "function" if prev != "abstract" => {
                stack.push(BlockFrame {
                    is_function: true,
                    vars: params_after(&line[end..], "parameter"),
                });
            }
            "constructor" | "operator" => stack.push(BlockFrame {
                is_function: true,
                vars: params_after(&line[end..], "parameter"),
            }),
            "destructor" => stack.push(BlockFrame { is_function: true, vars: Vec::new() }),
            "get" | "set" => {
                if prev.is_empty() || MEMBER_MODIFIERS.contains(&prev.as_str()) {
                    let after = &line[end..];
                    let trimmed = after.trim_start();
                    let name_len = trimmed
                        .find(|cc: char| !(cc.is_alphanumeric() || cc == '_'))
                        .unwrap_or(trimmed.len());
                    if name_len > 0 && trimmed[name_len..].trim_start().starts_with('(') {
                        stack.push(BlockFrame {
                            is_function: true,
                            vars: params_after(after, "parameter"),
                        });
                    }
                }
            }
            "do" => {
                if prev != "for" && prev != "while" && !saw_loop_word && !loop_awaiting_do {
                    stack.push(BlockFrame { is_function: false, vars: Vec::new() });
                }
                loop_awaiting_do = false;
            }
            "end" => {
                stack.pop();
            }
            _ => {}
        }
        if word == "for" || word == "while" {
            saw_loop_word = true;
            loop_awaiting_do = true;
        }
        prev = word.to_string();
    }
    carry.pending_loop = loop_awaiting_do;
}

fn params_after(after: &str, kind: &'static str) -> Vec<(String, &'static str)> {
    let Some(open) = after.find('(') else {
        return Vec::new();
    };
    let inner = &after[open + 1..];
    let mut depth = 0i32;
    let mut close = inner.len();
    let mut last_char = ' ';
    for (i, c) in inner.char_indices() {
        match c {
            '(' | '{' | '[' | '<' => depth += 1,
            ')' if depth == 0 => {
                close = i;
                break;
            }
            '>' if last_char == '-' => {}
            ')' | '}' | ']' | '>' => depth -= 1,
            _ => {}
        }
        last_char = c;
    }
    let list = &inner[..close];
    let mut out = Vec::new();
    let mut piece_start = 0;
    let mut piece_depth = 0i32;
    let bytes: Vec<(usize, char)> = list.char_indices().collect();
    let mut cuts: Vec<usize> = Vec::new();
    let mut prev_c = ' ';
    for (i, c) in &bytes {
        match c {
            '(' | '{' | '[' | '<' => piece_depth += 1,
            '>' if prev_c == '-' => {}
            ')' | '}' | ']' | '>' => piece_depth -= 1,
            ',' if piece_depth == 0 => cuts.push(*i),
            _ => {}
        }
        prev_c = *c;
    }
    cuts.push(list.len());
    for cut in cuts {
        let piece = list[piece_start..cut].trim();
        piece_start = cut + 1;
        let name_len = piece
            .find(|c: char| !(c.is_alphanumeric() || c == '_'))
            .unwrap_or(piece.len());
        let name = &piece[..name_len];
        if !name.is_empty() && name != "_" {
            out.push((name.to_string(), kind));
        }
    }
    out
}

fn loop_vars_after(after: &str) -> Vec<(String, &'static str)> {
    let mut end = after.len();
    if let Some(eq) = after.find('=') {
        end = end.min(eq);
    }
    if let Some(pos) = find_word(after, "in") {
        end = end.min(pos);
    }
    if let Some(pos) = find_word(after, "do") {
        end = end.min(pos);
    }
    after[..end]
        .split(',')
        .map(str::trim)
        .filter(|n| {
            !n.is_empty()
                && *n != "_"
                && n.chars().all(|c| c.is_alphanumeric() || c == '_')
        })
        .map(|n| (n.to_string(), "loop variable"))
        .collect()
}

fn find_word(text: &str, word: &str) -> Option<usize> {
    let mut from = 0;
    while let Some(pos) = text[from..].find(word) {
        let start = from + pos;
        let end = start + word.len();
        let before_ok = start == 0
            || text[..start]
                .chars()
                .last()
                .map(|c| !(c.is_alphanumeric() || c == '_'))
                .unwrap_or(true);
        let after_ok = text[end..]
            .chars()
            .next()
            .map(|c| !(c.is_alphanumeric() || c == '_'))
            .unwrap_or(true);
        if before_ok && after_ok {
            return Some(start);
        }
        from = end;
    }
    None
}

pub(crate) fn strip_strings_and_comments(line: &str) -> String {
    let chars: Vec<char> = line.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    let mut quote: Option<char> = None;
    while i < chars.len() {
        let c = chars[i];
        match quote {
            Some(q) => {
                if c == '\\' {
                    i += 1;
                } else if c == q {
                    quote = None;
                }
            }
            None => {
                if c == '"' || c == '\'' || c == '`' {
                    quote = Some(c);
                } else if c == '-' && chars.get(i + 1) == Some(&'-') {
                    break;
                } else {
                    out.push(c);
                }
            }
        }
        i += 1;
    }
    out
}

pub(crate) fn line_has_word(line: &str, word: &str) -> bool {
    let chars: Vec<char> = line.chars().collect();
    let target: Vec<char> = word.chars().collect();
    let n = target.len();
    let mut i = 0;
    while i + n <= chars.len() {
        if chars[i..i + n] == target[..] {
            let before_ok = i == 0 || !is_word_char(chars[i - 1]);
            let after_ok = i + n >= chars.len() || !is_word_char(chars[i + n]);
            if before_ok && after_ok {
                return true;
            }
        }
        i += 1;
    }
    false
}

fn class_name_on_line(cleaned: &str) -> Option<String> {
    let chars: Vec<char> = cleaned.chars().collect();
    let target: Vec<char> = "class".chars().collect();
    let mut i = 0;
    while i + 5 <= chars.len() {
        if chars[i..i + 5] == target[..] {
            let before_ok = i == 0 || !is_word_char(chars[i - 1]);
            let after_ok = i + 5 >= chars.len() || !is_word_char(chars[i + 5]);
            if before_ok && after_ok {
                let mut j = i + 5;
                while j < chars.len() && chars[j].is_whitespace() {
                    j += 1;
                }
                let start = j;
                while j < chars.len() && is_word_char(chars[j]) {
                    j += 1;
                }
                if j > start {
                    return Some(chars[start..j].iter().collect());
                }
            }
        }
        i += 1;
    }
    None
}

fn type_alias_on_line(cleaned: &str) -> bool {
    line_has_word(cleaned, "type") && cleaned.contains('=')
}

fn generics_between_angles(line: &str) -> Vec<String> {
    let Some(open) = line.find('<') else {
        return Vec::new();
    };
    let Some(close) = line[open..].find('>') else {
        return Vec::new();
    };
    line[open + 1..open + close]
        .split(',')
        .map(str::trim)
        .filter(|s| {
            !s.is_empty()
                && s.chars().next().map(|c| c.is_alphabetic() || c == '_').unwrap_or(false)
                && s.chars().all(is_word_char)
        })
        .map(str::to_string)
        .collect()
}

pub fn generic_params_in_scope(text: &str, line0: usize) -> Vec<String> {
    let lines: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    let start = line0.min(lines.len().saturating_sub(1));
    let lowest = start.saturating_sub(100);
    let mut i = start;
    loop {
        let cleaned = strip_strings_and_comments(lines.get(i).unwrap_or(&""));
        if line_has_word(&cleaned, "function") && cleaned.contains('<') {
            out.extend(generics_between_angles(&cleaned));
            break;
        }
        if type_alias_on_line(&cleaned) {
            if cleaned.contains('<') {
                out.extend(generics_between_angles(&cleaned));
            }
            break;
        }
        if line_has_word(&cleaned, "end") && i != start {
            break;
        }
        if i == lowest || i == 0 {
            break;
        }
        i -= 1;
    }
    out.sort();
    out.dedup();
    out
}

pub fn in_type_alias_body(text: &str, line0: usize, col0: usize) -> bool {
    let mut stack: Vec<bool> = Vec::new();
    for (i, raw) in text.lines().enumerate().take(line0 + 1) {
        let upto: String = if i == line0 {
            raw.chars().take(col0).collect()
        } else {
            raw.to_string()
        };
        let cleaned = strip_strings_and_comments(&upto);
        let is_type_line = type_alias_on_line(&cleaned);
        for c in cleaned.chars() {
            match c {
                '{' => {
                    let parent = stack.last().copied().unwrap_or(false);
                    stack.push(parent || is_type_line);
                }
                '}' => {
                    stack.pop();
                }
                _ => {}
            }
        }
        if i == line0 {
            break;
        }
    }
    stack.last().copied().unwrap_or(false)
}

pub fn enclosing_class(text: &str, line0: usize, col0: usize) -> Option<String> {
    let mut stack: Vec<Option<String>> = Vec::new();
    for (i, raw) in text.lines().enumerate().take(line0 + 1) {
        let upto: String = if i == line0 {
            raw.chars().take(col0).collect()
        } else {
            raw.to_string()
        };
        let cleaned = strip_strings_and_comments(&upto);
        let class_name = class_name_on_line(&cleaned);
        for c in cleaned.chars() {
            match c {
                '{' => {
                    let inside_class = matches!(stack.last(), Some(Some(_)));
                    if !inside_class {
                        stack.push(class_name.clone());
                    } else {
                        stack.push(None);
                    }
                }
                '}' => {
                    stack.pop();
                }
                _ => {}
            }
        }
        if i == line0 {
            break;
        }
    }
    stack.last().cloned().flatten()
}

fn class_member_completion(text: &str, line0: usize, col0: usize) -> Option<Vec<Item>> {
    let line = text.lines().nth(line0).unwrap_or("");
    let head: String = line.chars().take(col0).collect();
    let ends_mid_word = head.chars().last().map(is_word_char).unwrap_or(false);
    let mut tokens: Vec<&str> = head.split_whitespace().collect();
    if ends_mid_word {
        tokens.pop();
    }
    if !tokens.iter().all(|t| MEMBER_MODIFIERS.contains(t)) {
        return None;
    }
    enclosing_class(text, line0, col0)?;
    if inside_function_block(text, line0, col0) {
        return None;
    }
    let items = CLASS_MEMBER_KEYWORDS
        .iter()
        .filter(|kw| !tokens.contains(kw))
        .map(|kw| Item::plain(*kw, KIND_KEYWORD, "class member").prioritized())
        .collect();
    Some(items)
}

fn line_block_balance(stripped_line: &str) -> i32 {
    let mut balance = 0i32;
    let mut prev = String::new();
    let mut saw_loop_word = false;
    for word in stripped_line
        .split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .filter(|w| !w.is_empty())
    {
        match word {
            "if" | "for" | "while" | "switch" | "case" | "default" | "constructor"
            | "destructor" | "operator" => {
                balance += 1;
                if word == "for" || word == "while" {
                    saw_loop_word = true;
                }
            }
            "function" => {
                if prev != "abstract" {
                    balance += 1;
                }
            }
            "do" => {
                if prev != "for" && prev != "while" && !saw_loop_word {
                    balance += 1;
                }
            }
            "end" => balance -= 1,
            _ => {}
        }
        prev = word.to_string();
    }
    balance
}

fn branch_indent_edit(
    text: &str,
    cur_line: &str,
    line0: usize,
    col0: usize,
) -> Option<ExtraEdit> {
    let before: String = cur_line.chars().take(col0).collect();
    let partial = before.trim_start();
    if !partial.is_empty() && !"elseif".starts_with(partial) && !"else".starts_with(partial) {
        return None;
    }
    let cur_indent_len = before.chars().count() - partial.chars().count();
    let stripped = strip_to_code(text);
    let lines: Vec<&str> = stripped.lines().collect();
    let mut pending_ends = 0i32;
    for l in (0..line0.min(lines.len())).rev() {
        let s = lines[l];
        if s.trim().is_empty() {
            continue;
        }
        let st = s.trim_start();
        let first = st
            .split(|c: char| !(c.is_alphanumeric() || c == '_'))
            .find(|w| !w.is_empty())
            .unwrap_or("");
        let net = line_block_balance(s);
        if net <= -1 {
            pending_ends += -net;
            continue;
        }
        if net >= 1 {
            if pending_ends >= net {
                pending_ends -= net;
                continue;
            }
            if first != "if" {
                return None;
            }
            let indent: String = s.chars().take_while(|c| *c == ' ' || *c == '\t').collect();
            if indent.chars().count() == cur_indent_len {
                return None;
            }
            return Some(ExtraEdit {
                line0: line0 as u32,
                start_col: 0,
                end_col: cur_indent_len as u32,
                new_text: indent,
            });
        }
        if pending_ends == 0 && (first == "else" || first == "elseif") {
            let indent: String = s.chars().take_while(|c| *c == ' ' || *c == '\t').collect();
            if indent.chars().count() == cur_indent_len {
                return None;
            }
            return Some(ExtraEdit {
                line0: line0 as u32,
                start_col: 0,
                end_col: cur_indent_len as u32,
                new_text: indent,
            });
        }
    }
    None
}

fn literal_strings_of_type(ty: &Type) -> Vec<String> {
    match ty {
        Type::StringLit(s) => vec![s.clone()],
        Type::Union(parts) => parts.iter().flat_map(literal_strings_of_type).collect(),
        _ => Vec::new(),
    }
}

fn call_context(before: &str) -> Option<(String, usize)> {
    let chars: Vec<char> = before.chars().collect();
    let mut opens: Vec<usize> = Vec::new();
    for (i, c) in chars.iter().enumerate() {
        match c {
            '(' => opens.push(i),
            ')' => {
                opens.pop();
            }
            _ => {}
        }
    }
    let open = *opens.last()?;
    let mut depth = 0i32;
    let mut arg_idx = 0usize;
    for c in chars.iter().skip(open + 1) {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            ',' if depth == 0 => arg_idx += 1,
            _ => {}
        }
    }
    let callee: String = chars[..open].iter().collect();
    Some((callee.trim_end().to_string(), arg_idx))
}

fn call_argument_literals(
    view: &FileView,
    text: &str,
    before_string: &str,
    line0: usize,
    col0: usize,
) -> Option<Vec<Item>> {
    let (callee_text, arg_idx) = call_context(before_string)?;
    if callee_text.is_empty() {
        return None;
    }
    let colon_call = callee_text
        .char_indices()
        .rev()
        .find(|(_, c)| !(c.is_alphanumeric() || *c == '_'))
        .is_some_and(|(_, c)| c == ':');
    let probe = format!("{callee_text}.");
    let chain = chain_before(&probe, probe.chars().count())?;
    let self_class = enclosing_class(text, line0, col0);
    let ty = view.resolve_chain_in(&chain.segments, line0 as u32 + 1, self_class.as_deref())?;
    let param_ty = match &ty {
        Type::Function(Some(ft)) => {
            let skip_self = colon_call
                && ft.params.first().map(|p| p.name.as_str()) == Some("self");
            let idx = if skip_self { arg_idx + 1 } else { arg_idx };
            ft.params.get(idx).map(|p| p.ty.clone())
        }
        Type::Class(c) => view
            .find_class(c)
            .and_then(|info| info.constructor.as_ref())
            .and_then(|sig| sig.params.get(arg_idx).map(|p| p.ty.clone())),
        _ => None,
    }?;
    let lits = literal_strings_of_type(&param_ty);
    if lits.is_empty() {
        return None;
    }
    let mut lits = lits;
    lits.sort();
    lits.dedup();
    Some(
        lits.into_iter()
            .map(|s| Item::plain(s, KIND_CONSTANT, "literal"))
            .collect(),
    )
}

fn is_plain_ident(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_alphabetic() || c == '_' => {}
        _ => return false,
    }
    name.chars().all(is_word_char)
}

fn string_keys_of(view: &FileView, ty: &Type) -> Vec<(String, String)> {
    match ty {
        Type::Table(tt) => tt
            .fields
            .iter()
            .map(|(n, t)| (n.clone(), t.to_string()))
            .collect(),
        Type::Instance(c) => view
            .find_class(c)
            .map(|info| {
                info.fields
                    .iter()
                    .filter(|f| !f.is_static)
                    .map(|f| (f.name.clone(), f.ty.to_string()))
                    .collect()
            })
            .unwrap_or_default(),
        Type::Union(parts) => {
            let mut out = Vec::new();
            for p in parts {
                out.extend(string_keys_of(view, p));
            }
            out
        }
        _ => Vec::new(),
    }
}

fn bracket_key_completion(
    view: &FileView,
    text: &str,
    line: &str,
    line0: usize,
    col0: usize,
) -> Option<Vec<Item>> {
    let upto: String = line.chars().take(col0).collect();
    let (recv_text, quoted) = match in_open_string(line, col0) {
        Some((_, body)) => {
            let cut = upto.chars().count() - body.chars().count() - 1;
            let before: String = upto.chars().take(cut).collect();
            let trimmed = before.trim_end().to_string();
            (trimmed.strip_suffix('[')?.to_string(), true)
        }
        None => {
            let trimmed = upto.trim_end().to_string();
            (trimmed.strip_suffix('[')?.to_string(), false)
        }
    };
    let probe = format!("{recv_text}.");
    let chain = chain_before(&probe, probe.chars().count())?;
    let self_class = enclosing_class(text, line0, col0);
    let ty = view.resolve_chain_in(&chain.segments, line0 as u32 + 1, self_class.as_deref())?;
    let keys = string_keys_of(view, &ty);
    if keys.is_empty() {
        return None;
    }
    Some(
        keys.into_iter()
            .map(|(name, detail)| {
                let insert = if quoted {
                    None
                } else {
                    Some(format!("\"{name}\"]"))
                };
                Item {
                    label: name,
                    kind: KIND_FIELD,
                    detail,
                    insert_text: insert,
                    is_snippet: false,
                    sort_text: None,
                    auto_import: None,
                    extra_edit: None,
                }
            })
            .collect(),
    )
}

pub fn resolve_chain_full(
    view: &FileView,
    chain: &ChainAt,
    line: u32,
    self_class: Option<&str>,
) -> Option<Type> {
    if let Some(cast) = &chain.cast_base {
        let texpr = crate::type_syntax::parse_type(cast).ok()?;
        let mut ty = view.env.value_type(&texpr);
        for seg in &chain.segments {
            ty = view.member_type(&ty, &seg.name);
            if seg.called {
                ty = view.call_result(&ty);
            }
            if ty == Type::Unknown {
                return Some(Type::Unknown);
            }
        }
        return Some(ty);
    }
    view.resolve_chain_in(&chain.segments, line, self_class)
}

fn item_kind_for(ty: &Type) -> (i64, String) {
    let detail = ty.to_string();
    let kind = match ty {
        Type::Function(_) => KIND_FUNCTION,
        Type::Class(_) => KIND_CLASS,
        Type::Enum(_) => KIND_ENUM,
        Type::Interface(_) => KIND_INTERFACE,
        Type::Table(_) => KIND_MODULE,
        _ => KIND_VARIABLE,
    };
    (kind, detail)
}

fn switch_subject(text: &str, line0: usize) -> Option<String> {
    let lines: Vec<&str> = text.lines().collect();
    let mut i = line0.min(lines.len().saturating_sub(1));
    loop {
        let l = lines[i];
        if let Some(pos) = l.find("switch") {
            let after = &l[pos + 6..];
            let after = after.trim_start();
            if let Some(stripped) = after.strip_prefix('(') {
                let inner: String = stripped
                    .chars()
                    .take_while(|c| is_word_char(*c))
                    .collect();
                if !inner.is_empty() {
                    return Some(inner);
                }
            }
        }
        if i == 0 {
            return None;
        }
        i -= 1;
    }
}
