use luar_lsp::project::Project;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

static COUNTER: AtomicU32 = AtomicU32::new(0);

fn temp_project(files: &[(&str, &str)]) -> (PathBuf, Project) {
    let id = COUNTER.fetch_add(1, Ordering::SeqCst);
    let root = std::env::temp_dir().join(format!("luar_lines_{}_{}", std::process::id(), id));
    let _ = std::fs::remove_dir_all(&root);
    for (rel, content) in files {
        let path = root.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&path, content).unwrap();
    }
    let project = Project::load(&root);
    (root, project)
}

#[test]
fn class_member_param_diagnostics_point_at_the_member_line() {
    let src = "--#disable EmptyClass\n\nclass Destructable {\n    abstract function Destroy()\n        \n    end\n}\n\nfinal class Connection extends Destructable {\n    public SourceConnect:NotNil<any>\n\n    constructor(var:(...any) -> any)\n        \n    end\n}\n\nfinal class Signal extends Destructable {\n    public Connections:{Connection} = {}\n    public function Destroy()\n        for _, item in ipairs(self.Connections) do\n            item:Destroy()\n        end\n        self.Connections = {}\n    end\n}\n";
    let (root, p) = temp_project(&[("main.luar", src)]);
    let info = p.file(&root.join("main.luar")).unwrap();
    let unused = info
        .diagnostics
        .iter()
        .find(|d| d.message.contains("parameter 'var'"))
        .expect("expected UnusedParameter for 'var'");
    assert_eq!(unused.line, 12, "{:?}", info.diagnostics);
    assert!(
        !info
            .diagnostics
            .iter()
            .any(|d| d.line == 0 || d.line == 1),
        "no diagnostic should land on the directive line: {:?}",
        info.diagnostics
    );
}

#[test]
fn variadic_types_produce_no_syntax_errors() {
    let src = "type fun = (...any) -> ...any\nclass C {\n    public SourceConnect:NotNil<(...any) -> any>\n    public Handler:...any\n    constructor(var:...any)\n        print(var)\n    end\n}\n";
    let (root, p) = temp_project(&[("main.luar", src)]);
    let info = p.file(&root.join("main.luar")).unwrap();
    assert!(
        !info
            .diagnostics
            .iter()
            .any(|d| d.message.contains("expected") || d.severity == 1),
        "{:?}",
        info.diagnostics
    );
}
