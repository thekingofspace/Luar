use crate::annotations::{self, AnnotationSet};
use crate::infer::{Analysis, BindingKind, ClassInfo, EnumInfo, InferOptions};
use crate::json::Json;
use crate::resolve::TypeEnv;
use crate::type_syntax::{NameSeg, Param, TableTypeExpr, TypeExpr};
use crate::types::{TableType, Type};
use luar::ast::{Expr, Mutability, Stmt};
use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone)]
pub struct ModuleInfo {
    pub path: PathBuf,
    pub source: String,
    pub annotations: AnnotationSet,
    pub env: TypeEnv,
    pub analysis: Analysis,
    pub return_type: Type,
    pub diagnostics: Vec<crate::Diagnostic>,
    pub requires: Vec<PathBuf>,
    pub require_requests: Vec<(String, PathBuf)>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RequireTarget {
    Module(PathBuf),
    Directory(PathBuf, Vec<(String, PathBuf)>),
    Unresolved,
}

#[derive(Debug, Default)]
pub struct Project {
    pub root: PathBuf,
    pub aliases: HashMap<String, String>,
    pub files: HashMap<PathBuf, ModuleInfo>,
    pub luard_files: Vec<PathBuf>,
    pub config_luard: Vec<PathBuf>,
    pub luard_globals: Vec<(String, Type)>,
    pub luard_mutability: HashMap<String, Mutability>,
    pub luard_env: TypeEnv,
    pub luard_classes: HashMap<String, ClassInfo>,
    pub luard_enums: HashMap<String, EnumInfo>,
    pub pub_globals: Vec<(String, Type, PathBuf)>,
    pub pub_classes: HashMap<String, (ClassInfo, PathBuf)>,
    pub pub_enums: HashMap<String, (EnumInfo, PathBuf)>,
}

pub fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

struct ResolveCtx<'a> {
    root: &'a Path,
    files: &'a HashSet<PathBuf>,
    aliases: &'a HashMap<String, String>,
}

fn dir_listing(ctx: &ResolveCtx, dir: &Path, exclude: Option<&Path>) -> Vec<(String, PathBuf)> {
    let mut out: Vec<(String, PathBuf)> = Vec::new();
    let mut subdirs: HashSet<PathBuf> = HashSet::new();
    for f in ctx.files {
        if exclude.map(|e| e == f.as_path()).unwrap_or(false) {
            continue;
        }
        let Ok(rel) = f.strip_prefix(dir) else {
            continue;
        };
        let comps: Vec<_> = rel.components().collect();
        match comps.len() {
            1 => {
                let stem = f.file_stem().unwrap_or_default().to_string_lossy().to_string();
                if f.file_name().map(|n| n == "init.luar").unwrap_or(false) {
                    continue;
                }
                out.push((stem, f.clone()));
            }
            2 => {
                if comps[1].as_os_str() == "init.luar" {
                    let sub = dir.join(comps[0].as_os_str());
                    if subdirs.insert(sub.clone()) {
                        let name = comps[0].as_os_str().to_string_lossy().to_string();
                        out.push((name, sub.join("init.luar")));
                    }
                }
            }
            _ => {}
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

fn dir_exists(ctx: &ResolveCtx, dir: &Path) -> bool {
    ctx.files.iter().any(|f| f.strip_prefix(dir).is_ok())
}

fn resolve_path_target(ctx: &ResolveCtx, base: PathBuf, from: Option<&Path>) -> RequireTarget {
    let base = normalize(&base);
    let mut as_file = base.clone();
    let file_name = format!(
        "{}.luar",
        as_file.file_name().unwrap_or_default().to_string_lossy()
    );
    as_file.set_file_name(file_name);
    if ctx.files.contains(&as_file) {
        return RequireTarget::Module(as_file);
    }
    let init = base.join("init.luar");
    if ctx.files.contains(&init) {
        return RequireTarget::Module(init);
    }
    if dir_exists(ctx, &base) {
        return RequireTarget::Directory(base.clone(), dir_listing(ctx, &base, from));
    }
    RequireTarget::Unresolved
}

fn resolve_dir_request(ctx: &ResolveCtx, dir: PathBuf, from: Option<&Path>) -> RequireTarget {
    let dir = normalize(&dir);
    if dir_exists(ctx, &dir) {
        return RequireTarget::Directory(dir.clone(), dir_listing(ctx, &dir, from));
    }
    RequireTarget::Unresolved
}

fn resolve_require_ctx(ctx: &ResolveCtx, from: &Path, request: &str) -> RequireTarget {
    let from_dir = from.parent().unwrap_or(Path::new(""));
    if let Some(rest) = request.strip_prefix('@') {
        if rest == "self" || rest.starts_with("self/") {
            let sub = rest.strip_prefix("self").unwrap().trim_start_matches('/');
            let base = from_dir.to_path_buf();
            return if sub.is_empty() {
                resolve_dir_request(ctx, base, Some(from))
            } else {
                resolve_path_target(ctx, base.join(sub), Some(from))
            };
        }
        let (alias, sub) = match rest.find('/') {
            Some(i) => (&rest[..i], &rest[i + 1..]),
            None => (rest, ""),
        };
        let Some(target) = ctx.aliases.get(alias) else {
            return RequireTarget::Unresolved;
        };
        let cleaned = target.trim_start_matches("./").trim_end_matches('/');
        let base = if cleaned.is_empty() {
            ctx.root.to_path_buf()
        } else {
            ctx.root.join(cleaned)
        };
        return if sub.is_empty() {
            match resolve_path_target(ctx, base.clone(), Some(from)) {
                RequireTarget::Unresolved => resolve_dir_request(ctx, base, Some(from)),
                t => t,
            }
        } else {
            resolve_path_target(ctx, base.join(sub), Some(from))
        };
    }
    let dots = request.chars().take_while(|c| *c == '.').count();
    if dots > 0 {
        let after = &request[dots..];
        if !(after.is_empty() || after.starts_with('/')) {
            return RequireTarget::Unresolved;
        }
        let mut base = from_dir.to_path_buf();
        let init_extra = if from.file_name().map(|n| n == "init.luar").unwrap_or(false) {
            1
        } else {
            0
        };
        for _ in 0..(dots - 1 + init_extra) {
            base = base.parent().map(Path::to_path_buf).unwrap_or(base);
        }
        let rest = after.trim_start_matches('/');
        return if rest.is_empty() {
            resolve_dir_request(ctx, base, Some(from))
        } else {
            resolve_path_target(ctx, base.join(rest), Some(from))
        };
    }
    resolve_path_target(ctx, from_dir.join(request), Some(from))
}

fn require_type_ctx(
    ctx: &ResolveCtx,
    returns: &HashMap<PathBuf, Type>,
    from: &Path,
    request: &str,
) -> Option<Type> {
    match resolve_require_ctx(ctx, from, request) {
        RequireTarget::Module(path) => Some(returns.get(&path).cloned().unwrap_or(Type::Unknown)),
        RequireTarget::Directory(_, listing) => {
            let fields = listing
                .into_iter()
                .map(|(name, path)| {
                    (name, returns.get(&path).cloned().unwrap_or(Type::Unknown))
                })
                .collect();
            Some(Type::Table(TableType {
                fields,
                array: None,
                name: None,
            }))
        }
        RequireTarget::Unresolved => None,
    }
}

fn collect_files(dir: &Path, luar: &mut Vec<PathBuf>, luard: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if path.is_dir() {
            if matches!(
                name.as_str(),
                ".git" | "node_modules" | "target" | "out" | ".vscode"
            ) {
                continue;
            }
            collect_files(&path, luar, luard);
        } else if name.ends_with(".luar") {
            luar.push(normalize(&path));
        } else if name.ends_with(".luard") {
            luard.push(normalize(&path));
        }
    }
}

fn module_return_label(program: &[Stmt], ann: &AnnotationSet) -> Option<String> {
    let mut label = None;
    for s in program {
        if let Stmt::Return { values, .. } = s {
            if let Some(Expr::Name(n)) = values.first() {
                let from_ann = ann
                    .vars
                    .iter()
                    .filter(|((vn, _), _)| vn == n)
                    .max_by_key(|((_, l), _)| *l)
                    .and_then(|(_, t)| t.simple_name().map(str::to_string));
                label = Some(from_ann.unwrap_or_else(|| n.clone()));
            }
        }
    }
    label
}

fn label_module_return(ty: Type, program: &[Stmt], ann: &AnnotationSet) -> Type {
    match ty {
        Type::Table(mut tt) if tt.name.is_none() => {
            tt.name = module_return_label(program, ann);
            Type::Table(tt)
        }
        other => other,
    }
}

fn collect_requires_stmts(stmts: &[Stmt], out: &mut Vec<String>) {
    for s in stmts {
        match s {
            Stmt::Declare { inits, .. } => {
                for e in inits {
                    collect_requires_expr(e, out);
                }
            }
            Stmt::Assign { values, .. } => {
                for e in values {
                    collect_requires_expr(e, out);
                }
            }
            Stmt::Do(body) => collect_requires_stmts(body, out),
            Stmt::If {
                branches,
                else_block,
                ..
            } => {
                for (cond, body) in branches {
                    collect_requires_expr(cond, out);
                    collect_requires_stmts(body, out);
                }
                if let Some(body) = else_block {
                    collect_requires_stmts(body, out);
                }
            }
            Stmt::While { cond, body, .. } => {
                collect_requires_expr(cond, out);
                collect_requires_stmts(body, out);
            }
            Stmt::ForNumeric {
                start,
                stop,
                step,
                body,
                ..
            } => {
                collect_requires_expr(start, out);
                collect_requires_expr(stop, out);
                if let Some(e) = step {
                    collect_requires_expr(e, out);
                }
                collect_requires_stmts(body, out);
            }
            Stmt::ForIn { iters, body, .. } => {
                for e in iters {
                    collect_requires_expr(e, out);
                }
                collect_requires_stmts(body, out);
            }
            Stmt::Return { values, .. } => {
                for e in values {
                    collect_requires_expr(e, out);
                }
            }
            Stmt::Buff { init, .. } => collect_requires_expr(init, out),
            Stmt::Expr(e, _) => collect_requires_expr(e, out),
            Stmt::Class { members, .. } => {
                for m in members {
                    match m {
                        luar::ast::ClassMember::Field { default, .. } => {
                            if let Some(e) = default {
                                collect_requires_expr(e, out);
                            }
                        }
                        luar::ast::ClassMember::Method { func, .. }
                        | luar::ast::ClassMember::Getter { func, .. }
                        | luar::ast::ClassMember::Setter { func, .. }
                        | luar::ast::ClassMember::Constructor { func }
                        | luar::ast::ClassMember::Destructor { func }
                        | luar::ast::ClassMember::Operator { func, .. } => {
                            collect_requires_stmts(&func.body, out);
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

fn collect_requires_expr(e: &Expr, out: &mut Vec<String>) {
    match e {
        Expr::Call { callee, args } => {
            if let (Expr::Name(f), Some(Expr::Str(path))) = (callee.as_ref(), args.first()) {
                if f == "require" {
                    out.push(path.clone());
                }
            }
            collect_requires_expr(callee, out);
            for a in args {
                collect_requires_expr(a, out);
            }
        }
        Expr::MethodCall { receiver, args, .. } => {
            collect_requires_expr(receiver, out);
            for a in args {
                collect_requires_expr(a, out);
            }
        }
        Expr::Function { body, .. } => collect_requires_stmts(body, out),
        Expr::Table(entries) => {
            for entry in entries {
                match entry {
                    luar::ast::TableEntry::Positional(v) => collect_requires_expr(v, out),
                    luar::ast::TableEntry::Keyed { key, value } => {
                        collect_requires_expr(key, out);
                        collect_requires_expr(value, out);
                    }
                }
            }
        }
        Expr::Index { base, key } => {
            collect_requires_expr(base, out);
            collect_requires_expr(key, out);
        }
        Expr::Switch {
            subject,
            cases,
            default,
        } => {
            collect_requires_expr(subject, out);
            for case in cases {
                collect_requires_expr(&case.pattern, out);
                collect_requires_stmts(&case.body, out);
            }
            if let Some(body) = default {
                collect_requires_stmts(body, out);
            }
        }
        Expr::Unary { expr, .. } => collect_requires_expr(expr, out),
        Expr::Binary { lhs, rhs, .. } | Expr::Logical { lhs, rhs, .. } => {
            collect_requires_expr(lhs, out);
            collect_requires_expr(rhs, out);
        }
        _ => {}
    }
}

fn require_bindings(stmts: &[Stmt]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for s in stmts {
        if let Stmt::Declare { names, inits, .. } = s {
            if names.len() == 1 && inits.len() == 1 {
                if let Expr::Call { callee, args } = &inits[0] {
                    if let (Expr::Name(f), [Expr::Str(path)]) = (callee.as_ref(), args.as_slice())
                    {
                        if f == "require" {
                            out.push((names[0].clone(), path.clone()));
                        }
                    }
                }
            }
        }
    }
    out
}

impl Project {
    pub fn load(root: &Path) -> Project {
        let root = normalize(root);
        let mut luar_paths = Vec::new();
        let mut luard_paths = Vec::new();
        collect_files(&root, &mut luar_paths, &mut luard_paths);

        let mut project = Project {
            root: root.clone(),
            ..Project::default()
        };
        project.aliases = load_aliases(&root);
        project.luard_files = luard_paths.clone();
        project.config_luard = load_luard_imports(&root);
        for p in &project.config_luard {
            if !project.luard_files.contains(p) {
                project.luard_files.push(p.clone());
            }
        }

        let all_luard = project.luard_files.clone();
        for path in &all_luard {
            if let Ok(src) = std::fs::read_to_string(path) {
                project.add_luard_source(&src);
            }
        }

        let mut sources: Vec<(PathBuf, String)> = Vec::new();
        for path in &luar_paths {
            if let Ok(src) = std::fs::read_to_string(path) {
                sources.push((path.clone(), src));
            }
        }
        for (path, src) in &sources {
            project.analyze_file_pass1(path.clone(), src.clone());
        }
        project.link_and_repass();
        project
    }

    pub fn add_luard_source(&mut self, src: &str) {
        let Ok(program) = crate::parse_source_safe(src) else {
            return;
        };
        let ann = annotations::scan(src);
        let mut env = TypeEnv::from_program(&program);
        env.apply_annotations(&ann);
        let opts = InferOptions {
            annotations: Some(&ann),
            env: Some(&env),
            ambient: true,
            ..InferOptions::default()
        };
        let analysis = crate::identify_program_with(&program, &opts);
        for b in &analysis.bindings {
            if let Some(slot) = self.luard_globals.iter_mut().find(|(n, _)| *n == b.name) {
                slot.1 = b.ty.clone();
            } else {
                self.luard_globals.push((b.name.clone(), b.ty.clone()));
            }
            let mu = match b.kind {
                crate::infer::BindingKind::Declare { mutability, .. } => mutability,
                _ => Mutability::Mutable,
            };
            self.luard_mutability.insert(b.name.clone(), mu);
        }
        self.luard_classes.extend(analysis.classes.clone());
        self.luard_enums.extend(analysis.enums.clone());
        self.luard_env.merge_globals(&env);
    }

    pub fn rebuild_luard(&mut self, overlay: &HashMap<PathBuf, String>) {
        self.luard_globals.clear();
        self.luard_mutability.clear();
        self.luard_classes.clear();
        self.luard_enums.clear();
        self.luard_env = TypeEnv::default();
        let paths = self.luard_files.clone();
        for path in &paths {
            let src = match overlay.get(path) {
                Some(s) => s.clone(),
                None => match std::fs::read_to_string(path) {
                    Ok(s) => s,
                    Err(_) => continue,
                },
            };
            self.add_luard_source(&src);
        }
    }

    fn base_env_for(&self, program: &[Stmt], ann: &AnnotationSet) -> TypeEnv {
        let mut env = TypeEnv::from_program(program);
        env.apply_annotations(ann);
        env.merge_globals(&self.luard_env);
        env
    }

    fn analyze_file_pass1(&mut self, path: PathBuf, source: String) {
        let (program, parsed_src, diagnostics) =
            crate::parse_source_repaired_with_errors(&source);
        let previous = self.files.get(&path).map(|m| m.analysis.clone());
        let ann = annotations::scan(&parsed_src);
        let env = self.base_env_for(&program, &ann);
        let opts = InferOptions {
            globals: self.luard_globals.clone(),
            annotations: Some(&ann),
            env: Some(&env),
            classes: self.luard_classes.clone(),
            enums: self.luard_enums.clone(),
            ..InferOptions::default()
        };
        let mut analysis = crate::identify_program_with(&program, &opts);
        let return_type = label_module_return(
            analysis
                .module_returns
                .first()
                .cloned()
                .unwrap_or(Type::Nil),
            &program,
            &ann,
        );
        if parsed_src != source || program.is_empty() {
            if let Some(prev) = &previous {
                merge_stale_analysis(&mut analysis, prev);
            }
        }
        self.files.insert(
            path.clone(),
            ModuleInfo {
                path,
                source,
                annotations: ann,
                env,
                analysis,
                return_type,
                diagnostics,
                requires: Vec::new(),
                require_requests: Vec::new(),
            },
        );
    }

    fn file_index(&self) -> HashSet<PathBuf> {
        self.files.keys().cloned().collect()
    }

    fn return_snapshot(&self) -> HashMap<PathBuf, Type> {
        self.files
            .iter()
            .map(|(p, m)| (p.clone(), m.return_type.clone()))
            .collect()
    }

    fn collect_pub_exports(&mut self) {
        let mut globals: Vec<(String, Type, PathBuf)> = Vec::new();
        let mut classes: HashMap<String, (ClassInfo, PathBuf)> = HashMap::new();
        let mut enums: HashMap<String, (EnumInfo, PathBuf)> = HashMap::new();
        for (path, info) in &self.files {
            for b in &info.analysis.bindings {
                if let BindingKind::Declare {
                    visibility: luar::ast::Visibility::Pub,
                    ..
                } = b.kind
                {
                    match globals.iter_mut().find(|(n, _, _)| *n == b.name) {
                        Some(slot) => slot.1 = b.ty.clone(),
                        None => globals.push((b.name.clone(), b.ty.clone(), path.clone())),
                    }
                }
            }
            for (name, c) in &info.analysis.classes {
                if c.is_pub {
                    classes.insert(name.clone(), (c.clone(), path.clone()));
                    match globals.iter_mut().find(|(n, _, _)| n == name) {
                        Some(slot) => slot.1 = Type::Class(name.clone()),
                        None => globals.push((
                            name.clone(),
                            Type::Class(name.clone()),
                            path.clone(),
                        )),
                    }
                }
            }
            for (name, e) in &info.analysis.enums {
                if e.is_pub {
                    enums.insert(name.clone(), (e.clone(), path.clone()));
                    match globals.iter_mut().find(|(n, _, _)| n == name) {
                        Some(slot) => slot.1 = Type::Enum(name.clone()),
                        None => globals.push((
                            name.clone(),
                            Type::Enum(name.clone()),
                            path.clone(),
                        )),
                    }
                }
            }
        }
        self.pub_globals = globals;
        self.pub_classes = classes;
        self.pub_enums = enums;
    }

    pub fn link_and_repass(&mut self) {
        self.collect_pub_exports();
        let index = self.file_index();
        let returns = self.return_snapshot();
        let paths: Vec<PathBuf> = self.files.keys().cloned().collect();
        for path in paths {
            self.repass_file(&path, &index, &returns);
        }
        self.annotate_require_cycles();
    }

    fn repass_file(
        &mut self,
        path: &Path,
        index: &HashSet<PathBuf>,
        returns: &HashMap<PathBuf, Type>,
    ) {
        let Some(info) = self.files.get(path) else {
            return;
        };
        let source = info.source.clone();
        let (program, parsed_src, mut diagnostics) =
            crate::parse_source_repaired_with_errors(&source);
        let ann = annotations::scan(&parsed_src);
        let mut env = self.base_env_for(&program, &ann);

        let ctx = ResolveCtx {
            root: &self.root,
            files: index,
            aliases: &self.aliases,
        };
        let mut import_classes: HashMap<String, ClassInfo> = self.luard_classes.clone();
        let mut import_enums: HashMap<String, EnumInfo> = self.luard_enums.clone();
        let mut extra_globals = self.luard_globals.clone();
        for (name, ty, src_path) in &self.pub_globals {
            if src_path != path {
                extra_globals.push((name.clone(), ty.clone()));
            }
        }
        for (name, (c, src_path)) in &self.pub_classes {
            if src_path != path {
                import_classes.insert(name.clone(), c.clone());
                env.define_class(name);
            }
        }
        for (name, (e, src_path)) in &self.pub_enums {
            if src_path != path {
                import_enums.insert(name.clone(), e.clone());
                env.define_enum(name);
            }
        }
        for (local, request) in require_bindings(&program) {
            if let RequireTarget::Module(target) = resolve_require_ctx(&ctx, path, &request) {
                if let Some(target_info) = self.files.get(&target) {
                    env.define_module(&local, target_info.env.clone());
                    import_classes.extend(target_info.analysis.classes.clone());
                    import_enums.extend(target_info.analysis.enums.clone());
                }
            }
        }
        let mut requires: Vec<PathBuf> = Vec::new();
        let mut resolved_requires: Vec<(String, PathBuf)> = Vec::new();
        let mut all_requests: Vec<String> = Vec::new();
        collect_requires_stmts(&program, &mut all_requests);
        all_requests.dedup();
        for request in all_requests {
            if let RequireTarget::Module(target) = resolve_require_ctx(&ctx, path, &request) {
                if !requires.contains(&target) {
                    requires.push(target.clone());
                }
                resolved_requires.push((request, target));
            }
        }

        let hook = |request: &str| -> Option<Type> {
            require_type_ctx(&ctx, returns, path, request)
        };
        let opts = InferOptions {
            globals: extra_globals,
            annotations: Some(&ann),
            env: Some(&env),
            require: Some(&hook),
            classes: import_classes,
            enums: import_enums,
            ambient: false,
        };
        let mut analysis = crate::identify_program_with(&program, &opts);
        let return_type = label_module_return(
            analysis
                .module_returns
                .first()
                .cloned()
                .unwrap_or(Type::Nil),
            &program,
            &ann,
        );
        let mut ann_sites: Vec<(u32, &TypeExpr)> = Vec::new();
        for ((_, line), t) in &ann.vars {
            ann_sites.push((*line, t));
        }
        for ((_, line), t) in &ann.fn_returns {
            ann_sites.push((*line, t));
        }
        for ((_, line), params) in &ann.fn_params {
            for t in params.values() {
                ann_sites.push((*line, t));
            }
        }
        let directives = luar::ferrite::collect_directives(&source);
        for (line, t) in ann_sites {
            visit_named_segs(t, &mut |seg: &NameSeg| {
                if let Some(alias) = env.aliases.get(&seg.name) {
                    let expected = alias.generics.len();
                    let got = seg.args.as_ref().map(|a| a.len()).unwrap_or(0);
                    if expected != got && !directives.silences("GenericArity", line) {
                        diagnostics.push(crate::Diagnostic {
                            line,
                            col: 1,
                            message: format!(
                                "type '{}' expects {} generic argument(s), got {}",
                                seg.name, expected, got
                            ),
                            severity: 1,
                        });
                    }
                }
            });
        }
        for stmt in &program {
            if let Stmt::Enum {
                name,
                variants,
                line,
                ..
            } = stmt
            {
                let mut seen_variants: HashSet<&String> = HashSet::new();
                for (vname, _) in variants {
                    if !seen_variants.insert(vname) {
                        let dup_line =
                            find_nth_word_line(&source, vname, 2, *line).unwrap_or(*line);
                        if directives.silences("DuplicateEnumVariant", dup_line) {
                            continue;
                        }
                        diagnostics.push(crate::Diagnostic {
                            line: dup_line,
                            col: 1,
                            message: format!(
                                "duplicate variant '{vname}' in enum '{name}'"
                            ),
                            severity: 1,
                        });
                    }
                }
            }
        }
        let luard_muts: Vec<(String, Mutability)> = self
            .luard_mutability
            .iter()
            .map(|(n, m)| (n.clone(), *m))
            .collect();
        let mut shadowed: HashSet<&str> = HashSet::new();
        for b in &analysis.bindings {
            if matches!(
                b.kind,
                BindingKind::Declare { .. } | BindingKind::LoopVar
            ) {
                shadowed.insert(&b.name);
            }
            if let Type::Function(Some(sig)) = &b.ty {
                for p in &sig.params {
                    shadowed.insert(p.name.as_str());
                }
            }
        }
        let mut mutability: HashMap<&str, Mutability> = HashMap::new();
        for (n, m) in &luard_muts {
            if !shadowed.contains(n.as_str()) {
                mutability.insert(n.as_str(), *m);
            }
        }
        for b in &analysis.bindings {
            match b.kind {
                BindingKind::Declare {
                    mutability: m, ..
                } => {
                    mutability.insert(&b.name, m);
                }
                BindingKind::BareAssign => {
                    mutability.insert(&b.name, Mutability::Const);
                }
                BindingKind::Assign => {
                    if mutability.get(b.name.as_str()) == Some(&Mutability::Const)
                        && b.ty != Type::Nil
                        && !directives.silences("MutateImmutable", b.line.unwrap_or(1))
                    {
                        diagnostics.push(crate::Diagnostic {
                            line: b.line.unwrap_or(1),
                            col: 1,
                            message: format!(
                                "cannot reassign immutable '{}' (only nil is allowed, which frees it)",
                                b.name
                            ),
                            severity: 1,
                        });
                    }
                }
                _ => {}
            }
        }
        if path.extension().map(|e| e == "luar").unwrap_or(false) {
            for d in luar::ferrite::check(&source) {
                if d.code == "SyntaxError" || d.code == "MutateImmutable" {
                    continue;
                }
                diagnostics.push(crate::Diagnostic {
                    line: d.line,
                    col: 1,
                    message: format!("{} [{}]", d.message, d.code),
                    severity: match d.severity {
                        luar::ferrite::Severity::Error => 1,
                        luar::ferrite::Severity::Warning => 2,
                    },
                });
            }
        }
        check_notnil(&program, &ann, &analysis, &directives, &mut diagnostics);
        for class in analysis.classes.values() {
            let mut unknown: Vec<&String> = Vec::new();
            if let Some(parent) = &class.parent {
                if !self.class_known(&analysis, parent) {
                    unknown.push(parent);
                }
            }
            for mixin in &class.mixins {
                if !self.class_known(&analysis, mixin) {
                    unknown.push(mixin);
                }
            }
            for name in unknown {
                let line = find_word_line(&source, name).unwrap_or(1);
                if directives.silences("UnknownClass", line) {
                    continue;
                }
                diagnostics.push(crate::Diagnostic {
                    line,
                    col: 1,
                    message: format!(
                        "unknown class '{name}' (class names are case-sensitive)"
                    ),
                    severity: 2,
                });
            }
        }
        for class in analysis.classes.values() {
            let Some(parent_name) = &class.parent else {
                continue;
            };
            let class_line = find_word_line(&source, &class.name).unwrap_or(1);
            if let Some(parent) = self.find_class_info(&analysis, parent_name) {
                if parent.is_final {
                    let line = find_nth_word_line(&source, parent_name, 1, class_line)
                        .unwrap_or(class_line);
                    if !directives.silences("FinalOverride", line) {
                        diagnostics.push(crate::Diagnostic {
                            line,
                            col: 1,
                            message: format!(
                                "cannot extend final class '{parent_name}' — this errors at runtime [FinalOverride]"
                            ),
                            severity: 1,
                        });
                    }
                }
            }
            for m in &class.methods {
                let Some(owner) = self.final_method_owner(&analysis, parent_name, &m.name)
                else {
                    continue;
                };
                let line = find_nth_word_line(&source, &m.name, 1, class_line)
                    .unwrap_or(class_line);
                if directives.silences("FinalOverride", line) {
                    continue;
                }
                diagnostics.push(crate::Diagnostic {
                    line,
                    col: 1,
                    message: format!(
                        "cannot override final method '{}' from class '{owner}' — this errors at runtime [FinalOverride]",
                        m.name
                    ),
                    severity: 1,
                });
            }
        }
        if parsed_src != source || program.is_empty() {
            if let Some(prev) = self.files.get(path) {
                merge_stale_analysis(&mut analysis, &prev.analysis);
            }
        }
        if let Some(slot) = self.files.get_mut(path) {
            slot.annotations = ann;
            slot.env = env;
            slot.analysis = analysis;
            slot.return_type = return_type;
            slot.diagnostics = diagnostics;
            slot.requires = requires;
            slot.require_requests = resolved_requires;
        }
    }

    fn annotate_require_cycles(&mut self) {
        let paths: Vec<PathBuf> = self.files.keys().cloned().collect();
        for path in paths {
            let Some(info) = self.files.get(&path) else {
                continue;
            };
            let source = info.source.clone();
            let reqs = info.require_requests.clone();
            let directives = luar::ferrite::collect_directives(&source);
            let mut cycle_diags: Vec<crate::Diagnostic> = Vec::new();
            for (request, target) in &reqs {
                if let Some(chain) = self.find_require_cycle(target, &path) {
                    let line = find_line_containing(&source, &format!("\"{request}\""))
                        .or_else(|| find_line_containing(&source, &format!("'{request}'")))
                        .unwrap_or(1);
                    if directives.silences("RequireCycle", line) {
                        continue;
                    }
                    cycle_diags.push(crate::Diagnostic {
                        line,
                        col: 1,
                        message: format!(
                            "require(\"{request}\") creates a require cycle ({}) — this errors at runtime",
                            chain.join(" -> ")
                        ),
                        severity: 2,
                    });
                }
            }
            if let Some(slot) = self.files.get_mut(&path) {
                slot.diagnostics
                    .retain(|d| !d.message.contains("require cycle"));
                slot.diagnostics.extend(cycle_diags);
            }
        }
    }

    fn find_require_cycle(&self, start: &Path, me: &Path) -> Option<Vec<String>> {
        let stem = |p: &Path| {
            p.file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| p.to_string_lossy().into_owned())
        };
        if start == me {
            return Some(vec![stem(me), stem(me)]);
        }
        let mut visited: HashSet<PathBuf> = HashSet::new();
        let mut stack: Vec<(PathBuf, Vec<PathBuf>)> =
            vec![(start.to_path_buf(), vec![start.to_path_buf()])];
        while let Some((cur, chain)) = stack.pop() {
            if !visited.insert(cur.clone()) {
                continue;
            }
            let Some(info) = self.files.get(&cur) else {
                continue;
            };
            for next in &info.requires {
                if next == me {
                    let mut names = vec![stem(me)];
                    names.extend(chain.iter().map(|p| stem(p)));
                    names.push(stem(me));
                    return Some(names);
                }
                if !visited.contains(next) {
                    let mut c = chain.clone();
                    c.push(next.clone());
                    stack.push((next.clone(), c));
                }
            }
        }
        None
    }

    fn class_known(&self, analysis: &Analysis, name: &str) -> bool {
        analysis.classes.contains_key(name)
            || self.luard_classes.contains_key(name)
            || self
                .files
                .values()
                .any(|m| m.analysis.classes.contains_key(name))
    }

    fn find_class_info<'a>(&'a self, analysis: &'a Analysis, name: &str) -> Option<&'a ClassInfo> {
        analysis
            .classes
            .get(name)
            .or_else(|| self.luard_classes.get(name))
            .or_else(|| {
                self.files
                    .values()
                    .find_map(|m| m.analysis.classes.get(name))
            })
    }

    fn final_method_owner(
        &self,
        analysis: &Analysis,
        start: &str,
        method: &str,
    ) -> Option<String> {
        let mut seen: HashSet<String> = HashSet::new();
        let mut cur = start.to_string();
        while seen.insert(cur.clone()) {
            let info = self.find_class_info(analysis, &cur)?;
            if info
                .methods
                .iter()
                .any(|m| m.name == method && m.is_final)
            {
                return Some(info.name.clone());
            }
            cur = info.parent.clone()?;
        }
        None
    }

    pub fn reload_aliases(&mut self) {
        self.aliases = load_aliases(&self.root);
        let new_imports = load_luard_imports(&self.root);
        if new_imports != self.config_luard {
            let old = std::mem::replace(&mut self.config_luard, new_imports);
            self.luard_files.retain(|p| !old.contains(p));
            for p in &self.config_luard {
                if !self.luard_files.contains(p) {
                    self.luard_files.push(p.clone());
                }
            }
            self.rebuild_luard(&HashMap::new());
        }
        self.link_and_repass();
    }

    pub fn update_file(&mut self, path: &Path, source: String) {
        let path = normalize(path);
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        if name.ends_with(".luard") {
            if !self.luard_files.contains(&path) {
                self.luard_files.push(path.clone());
            }
            let mut overlay = HashMap::new();
            overlay.insert(path.clone(), source);
            self.rebuild_luard(&overlay);
            self.link_and_repass();
            return;
        }
        self.analyze_file_pass1(path.clone(), source);
        let before = (
            self.pub_globals.clone(),
            self.pub_classes.clone(),
            self.pub_enums.clone(),
        );
        self.collect_pub_exports();
        if before.0 != self.pub_globals
            || before.1 != self.pub_classes
            || before.2 != self.pub_enums
        {
            self.link_and_repass();
            return;
        }
        let index = self.file_index();
        let returns = self.return_snapshot();
        self.repass_file(&path, &index, &returns);
        let dependents: Vec<PathBuf> = self
            .files
            .iter()
            .filter(|(p, m)| **p != path && m.requires.contains(&path))
            .map(|(p, _)| p.clone())
            .collect();
        let returns = self.return_snapshot();
        for dep in dependents {
            self.repass_file(&dep, &index, &returns);
        }
        self.annotate_require_cycles();
    }

    pub fn file(&self, path: &Path) -> Option<&ModuleInfo> {
        self.files.get(&normalize(path))
    }

    pub fn resolve_require(&self, from: &Path, request: &str) -> RequireTarget {
        let index = self.file_index();
        let ctx = ResolveCtx {
            root: &self.root,
            files: &index,
            aliases: &self.aliases,
        };
        resolve_require_ctx(&ctx, &normalize(from), request)
    }

    pub fn require_type(&self, from: &Path, request: &str) -> Type {
        let index = self.file_index();
        let returns = self.return_snapshot();
        let ctx = ResolveCtx {
            root: &self.root,
            files: &index,
            aliases: &self.aliases,
        };
        require_type_ctx(&ctx, &returns, &normalize(from), request).unwrap_or(Type::Unknown)
    }

    pub fn complete_require(&self, from: &Path, partial: &str) -> Vec<String> {
        let from = normalize(from);
        let index = self.file_index();
        let ctx = ResolveCtx {
            root: &self.root,
            files: &index,
            aliases: &self.aliases,
        };
        if partial.starts_with('@') {
            let body = &partial[1..];
            if !body.contains('/') {
                let mut out: Vec<String> = self
                    .aliases
                    .keys()
                    .map(|a| format!("@{a}"))
                    .collect();
                if from.file_name().map(|n| n == "init.luar").unwrap_or(false) {
                    out.push("@self".to_string());
                }
                out.sort();
                return out;
            }
        }
        let (dir_part, _) = match partial.rfind('/') {
            Some(i) => (&partial[..i + 1], &partial[i + 1..]),
            None => ("./", partial),
        };
        match resolve_require_ctx(&ctx, &from, dir_part) {
            RequireTarget::Directory(_, listing) => {
                listing.into_iter().map(|(name, _)| name).collect()
            }
            RequireTarget::Module(_) | RequireTarget::Unresolved => Vec::new(),
        }
    }
}

fn merge_stale_analysis(current: &mut Analysis, previous: &Analysis) {
    for prev in &previous.bindings {
        if !current.bindings.iter().any(|b| b.name == prev.name) {
            current.bindings.push(prev.clone());
        }
    }
    for (name, info) in &previous.classes {
        current
            .classes
            .entry(name.clone())
            .or_insert_with(|| info.clone());
    }
    for (name, info) in &previous.enums {
        current
            .enums
            .entry(name.clone())
            .or_insert_with(|| info.clone());
    }
    for (name, members) in &previous.interfaces {
        current
            .interfaces
            .entry(name.clone())
            .or_insert_with(|| members.clone());
    }
    for (name, ty) in &previous.aliases {
        if !current.aliases.iter().any(|(n, _)| n == name) {
            current.aliases.push((name.clone(), ty.clone()));
        }
    }
}

fn visit_named_segs<'a>(t: &'a TypeExpr, f: &mut impl FnMut(&'a NameSeg)) {
    match t {
        TypeExpr::Named(segs) => {
            for seg in segs {
                f(seg);
                if let Some(args) = &seg.args {
                    for a in args {
                        visit_named_segs(a, f);
                    }
                }
            }
        }
        TypeExpr::Optional(inner) => visit_named_segs(inner, f),
        TypeExpr::Union(parts) | TypeExpr::Intersection(parts) | TypeExpr::Tuple(parts) => {
            for p in parts {
                visit_named_segs(p, f);
            }
        }
        TypeExpr::Table(tt) => match tt {
            TableTypeExpr::Record(fields) => {
                for field in fields {
                    visit_named_segs(&field.ty, f);
                }
            }
            TableTypeExpr::Indexer { key, value } => {
                visit_named_segs(key, f);
                visit_named_segs(value, f);
            }
            TableTypeExpr::Array(elem) => visit_named_segs(elem, f),
            TableTypeExpr::Empty => {}
        },
        TypeExpr::Function { params, ret } => {
            for p in params {
                match p {
                    Param::Positional { ty, .. } => visit_named_segs(ty, f),
                    Param::Vararg { ty: Some(t) } => visit_named_segs(t, f),
                    Param::Vararg { ty: None } => {}
                }
            }
            visit_named_segs(ret, f);
        }
        TypeExpr::StringLit(_) | TypeExpr::NumberLit(_) => {}
    }
}

fn find_line_containing(source: &str, needle: &str) -> Option<u32> {
    source
        .lines()
        .position(|l| l.contains(needle))
        .map(|i| i as u32 + 1)
}

fn find_nth_word_line(source: &str, word: &str, nth: usize, from_line: u32) -> Option<u32> {
    let target: Vec<char> = word.chars().collect();
    let mut count = 0;
    for (i, line) in source.lines().enumerate() {
        if (i as u32 + 1) < from_line {
            continue;
        }
        let chars: Vec<char> = line.chars().collect();
        let n = target.len();
        let mut j = 0;
        while j + n <= chars.len() {
            if chars[j..j + n] == target[..] {
                let before_ok =
                    j == 0 || !(chars[j - 1].is_alphanumeric() || chars[j - 1] == '_');
                let after = j + n;
                let after_ok = after >= chars.len()
                    || !(chars[after].is_alphanumeric() || chars[after] == '_');
                if before_ok && after_ok {
                    count += 1;
                    if count == nth {
                        return Some(i as u32 + 1);
                    }
                }
            }
            j += 1;
        }
    }
    None
}

fn find_word_line(source: &str, word: &str) -> Option<u32> {
    let target: Vec<char> = word.chars().collect();
    for (i, line) in source.lines().enumerate() {
        let chars: Vec<char> = line.chars().collect();
        let n = target.len();
        let mut j = 0;
        while j + n <= chars.len() {
            if chars[j..j + n] == target[..] {
                let before_ok =
                    j == 0 || !(chars[j - 1].is_alphanumeric() || chars[j - 1] == '_');
                let after = j + n;
                let after_ok = after >= chars.len()
                    || !(chars[after].is_alphanumeric() || chars[after] == '_');
                if before_ok && after_ok {
                    return Some(i as u32 + 1);
                }
            }
            j += 1;
        }
    }
    None
}

fn is_notnil_expr(t: &TypeExpr) -> bool {
    matches!(t, TypeExpr::Named(segs) if segs.len() == 1
        && (segs[0].name == "NotNil" || segs[0].name == "notnil")
        && segs[0].args.is_some())
}

struct NotNilCheck<'a> {
    vars: std::collections::HashSet<String>,
    fn_params: HashMap<String, std::collections::HashSet<String>>,
    ann: &'a AnnotationSet,
    analysis: &'a crate::infer::Analysis,
    directives: &'a luar::ferrite::Directives,
    out: Vec<crate::Diagnostic>,
    current_line: u32,
}

impl<'a> NotNilCheck<'a> {
    fn error(&mut self, line: u32, message: String) {
        if self.directives.silences("NotNil", line) {
            return;
        }
        self.out.push(crate::Diagnostic {
            line,
            col: 1,
            message: format!("{message} [NotNil]"),
            severity: 1,
        });
    }

    fn notnil_param_positions(&self, fname: &str) -> Vec<(usize, String)> {
        let Some(param_set) = self.fn_params.get(fname) else {
            return Vec::new();
        };
        let Some(binding) = self.analysis.binding(fname) else {
            return Vec::new();
        };
        let Type::Function(Some(sig)) = &binding.ty else {
            return Vec::new();
        };
        sig.params
            .iter()
            .enumerate()
            .filter(|(_, p)| param_set.contains(&p.name))
            .map(|(i, p)| (i, p.name.clone()))
            .collect()
    }

    fn walk_block(&mut self, stmts: &[Stmt]) {
        for s in stmts {
            self.walk_stmt(s);
        }
    }

    fn walk_stmt(&mut self, stmt: &Stmt) {
        use luar::ast::{AssignOp, LValue};
        match stmt {
            Stmt::Declare { names, inits, line, .. } => {
                self.current_line = *line;
                for (i, name) in names.iter().enumerate() {
                    let here = self
                        .ann
                        .vars
                        .get(&(name.clone(), *line))
                        .map(is_notnil_expr)
                        .unwrap_or(false);
                    let multi = i >= inits.len()
                        && matches!(
                            inits.last(),
                            Some(luar::ast::Expr::Call { .. })
                                | Some(luar::ast::Expr::MethodCall { .. })
                                | Some(luar::ast::Expr::Vararg)
                        );
                    if here && !multi && matches!(inits.get(i), None | Some(luar::ast::Expr::Nil))
                    {
                        self.error(
                            *line,
                            format!(
                                "'{name}' is typed NotNil and must be initialized with a non-nil value"
                            ),
                        );
                    }
                }
                for e in inits {
                    self.walk_expr(e);
                }
            }
            Stmt::Assign { targets, op, values, line } => {
                self.current_line = *line;
                if *op == AssignOp::Assign {
                    for (i, t) in targets.iter().enumerate() {
                        if let LValue::Name(n) = t {
                            if self.vars.contains(n)
                                && matches!(values.get(i), Some(luar::ast::Expr::Nil))
                            {
                                self.error(
                                    *line,
                                    format!("cannot set '{n}' to nil ('{n}' is typed NotNil)"),
                                );
                            }
                        }
                    }
                }
                for v in values {
                    self.walk_expr(v);
                }
            }
            Stmt::Do(body) => self.walk_block(body),
            Stmt::If { branches, else_block, line } => {
                self.current_line = *line;
                for (c, b) in branches {
                    self.walk_expr(c);
                    self.walk_block(b);
                }
                if let Some(b) = else_block {
                    self.walk_block(b);
                }
            }
            Stmt::While { cond, body, line } => {
                self.current_line = *line;
                self.walk_expr(cond);
                self.walk_block(body);
            }
            Stmt::ForNumeric { start, stop, step, body, line, .. } => {
                self.current_line = *line;
                self.walk_expr(start);
                self.walk_expr(stop);
                if let Some(s) = step {
                    self.walk_expr(s);
                }
                self.walk_block(body);
            }
            Stmt::ForIn { iters, body, line, .. } => {
                self.current_line = *line;
                for e in iters {
                    self.walk_expr(e);
                }
                self.walk_block(body);
            }
            Stmt::Return { values, line } => {
                self.current_line = *line;
                for e in values {
                    self.walk_expr(e);
                }
            }
            Stmt::Expr(e, line) => {
                self.current_line = *line;
                self.walk_expr(e);
            }
            Stmt::Class { members, .. } => {
                for m in members {
                    match m {
                        luar::ast::ClassMember::Field { default: Some(e), .. } => self.walk_expr(e),
                        luar::ast::ClassMember::Method { func, .. }
                        | luar::ast::ClassMember::Constructor { func }
                        | luar::ast::ClassMember::Destructor { func }
                        | luar::ast::ClassMember::Operator { func, .. }
                        | luar::ast::ClassMember::Getter { func, .. }
                        | luar::ast::ClassMember::Setter { func, .. } => self.walk_block(&func.body),
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    fn walk_expr(&mut self, e: &luar::ast::Expr) {
        use luar::ast::Expr;
        match e {
            Expr::Call { callee, args } => {
                if let Expr::Name(f) = callee.as_ref() {
                    for (pos, pname) in self.notnil_param_positions(f) {
                        if matches!(args.get(pos), Some(Expr::Nil)) {
                            let line = self.current_line;
                            self.error(
                                line,
                                format!(
                                    "argument {} to '{f}' cannot be nil (parameter '{pname}' is typed NotNil)",
                                    pos + 1
                                ),
                            );
                        }
                    }
                }
                self.walk_expr(callee);
                for a in args {
                    self.walk_expr(a);
                }
            }
            Expr::MethodCall { receiver, args, .. } => {
                self.walk_expr(receiver);
                for a in args {
                    self.walk_expr(a);
                }
            }
            Expr::Index { base, key } => {
                self.walk_expr(base);
                self.walk_expr(key);
            }
            Expr::Function { body, .. } => self.walk_block(body),
            Expr::Table(entries) => {
                for entry in entries {
                    match entry {
                        luar::ast::TableEntry::Positional(v) => self.walk_expr(v),
                        luar::ast::TableEntry::Keyed { key, value } => {
                            self.walk_expr(key);
                            self.walk_expr(value);
                        }
                    }
                }
            }
            Expr::Switch { subject, cases, default } => {
                self.walk_expr(subject);
                for c in cases {
                    self.walk_expr(&c.pattern);
                    self.walk_block(&c.body);
                }
                if let Some(d) = default {
                    self.walk_block(d);
                }
            }
            Expr::Unary { expr, .. } => self.walk_expr(expr),
            Expr::Binary { lhs, rhs, .. } => {
                self.walk_expr(lhs);
                self.walk_expr(rhs);
            }
            Expr::Logical { lhs, rhs, .. } => {
                self.walk_expr(lhs);
                self.walk_expr(rhs);
            }
            _ => {}
        }
    }
}

fn check_notnil(
    program: &[Stmt],
    ann: &AnnotationSet,
    analysis: &crate::infer::Analysis,
    directives: &luar::ferrite::Directives,
    diagnostics: &mut Vec<crate::Diagnostic>,
) {
    let vars: std::collections::HashSet<String> = ann
        .vars
        .iter()
        .filter(|(_, t)| is_notnil_expr(t))
        .map(|((n, _), _)| n.clone())
        .collect();
    let mut fn_params: HashMap<String, std::collections::HashSet<String>> = HashMap::new();
    for ((f, _), params) in &ann.fn_params {
        for (p, t) in params {
            if is_notnil_expr(t) {
                fn_params.entry(f.clone()).or_default().insert(p.clone());
            }
        }
    }
    if vars.is_empty() && fn_params.is_empty() {
        return;
    }
    let mut check = NotNilCheck {
        vars,
        fn_params,
        ann,
        analysis,
        directives,
        out: Vec::new(),
        current_line: 1,
    };
    check.walk_block(program);
    diagnostics.append(&mut check.out);
}

fn load_luard_imports(root: &Path) -> Vec<PathBuf> {
    for candidate in ["luari.json", "luari", ".luari"] {
        let path = root.join(candidate);
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(json) = Json::parse(&text) else {
            continue;
        };
        let Some(entry) = json.get("luard") else {
            continue;
        };
        let raw: Vec<String> = match entry.as_array() {
            Some(items) => items
                .iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect(),
            None => entry.as_str().map(str::to_string).into_iter().collect(),
        };
        let mut out = Vec::new();
        for r in raw {
            let p = Path::new(&r);
            let resolved = if p.is_absolute() {
                normalize(p)
            } else {
                normalize(&root.join(p))
            };
            if !out.contains(&resolved) {
                out.push(resolved);
            }
        }
        return out;
    }
    Vec::new()
}

fn load_aliases(root: &Path) -> HashMap<String, String> {
    for candidate in ["luari.json", "luari", ".luari"] {
        let path = root.join(candidate);
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(json) = Json::parse(&text) else {
            continue;
        };
        let Some(map) = json.get("aliases").and_then(|a| a.as_object()) else {
            continue;
        };
        return map
            .iter()
            .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
            .collect();
    }
    HashMap::new()
}
