use luar_lsp::completion::{self, FileView};
use luar_lsp::project::Project;
use luar_lsp::types::Type;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

static COUNTER: AtomicU32 = AtomicU32::new(0);

fn temp_project(files: &[(&str, &str)]) -> (PathBuf, Project) {
    let id = COUNTER.fetch_add(1, Ordering::SeqCst);
    let root = std::env::temp_dir().join(format!("luar_oop_{}_{}", std::process::id(), id));
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

fn labels_at(p: &Project, path: &PathBuf, src: &str, line0: usize, col0: usize) -> Vec<String> {
    let view = FileView::from_project(p, path).expect("file");
    completion::complete(&view, src, line0, col0)
        .into_iter()
        .map(|i| i.label)
        .collect()
}

#[test]
fn user_inheritance_repro() {
    let src = "class test {\n    public name:string = \"Test\"\n}\n\nclass NewTest extends test {\n\n}\n\nlocal var = NewTest()\n\nNewTest.";
    let (root, p) = temp_project(&[("main.luar", src)]);
    let main = root.join("main.luar");
    let line0 = src.lines().count() - 1;
    let names = labels_at(&p, &main, src, line0, 8);
    assert!(
        names.contains(&"name".to_string()),
        "class-dot inherited member missing: {names:?}"
    );
    let with_var = format!("{src}\nvar.");
    let (root2, p2) = temp_project(&[("main.luar", &with_var)]);
    let main2 = root2.join("main.luar");
    let line0 = with_var.lines().count() - 1;
    let names = labels_at(&p2, &main2, &with_var, line0, 4);
    assert!(
        names.contains(&"name".to_string()),
        "instance inherited field missing: {names:?}"
    );
}

#[test]
fn mixin_fields_and_methods_in_completion() {
    let src = "class Mix {\n    public extra:number = 1\n    public function mixed()\n    end\n}\nclass Base {\n    public own:string = \"\"\n}\nclass Kid extends Base mixin Mix {\n}\nlocal k = Kid()\nk.";
    let (root, p) = temp_project(&[("main.luar", src)]);
    let main = root.join("main.luar");
    let line0 = src.lines().count() - 1;
    let names = labels_at(&p, &main, src, line0, 2);
    assert!(names.contains(&"own".to_string()), "{names:?}");
    assert!(names.contains(&"extra".to_string()), "mixin field missing: {names:?}");
    let colon = format!("{}k:", src.trim_end_matches("k."));
    let (root2, p2) = temp_project(&[("main.luar", &colon)]);
    let main2 = root2.join("main.luar");
    let line0 = colon.lines().count() - 1;
    let names = labels_at(&p2, &main2, &colon, line0, 2);
    assert!(names.contains(&"mixed".to_string()), "mixin method missing: {names:?}");
}

#[test]
fn override_method_wins() {
    let src = "class Animal {\n    public function speak(): string\n        return \"...\"\n    end\n}\nclass Dog extends Animal {\n    override function speak(): number\n        return 1\n    end\n}\nlocal d = Dog()\nlocal s = d:speak()\n";
    let (root, p) = temp_project(&[("main.luar", src)]);
    let main = root.join("main.luar");
    let info = p.file(&main).unwrap();
    assert_eq!(info.analysis.type_of("s"), Some(&Type::Number));
}

#[test]
fn interface_members_complete() {
    let src = "interface Pet { name describe }\nlocal p: Pet = adopt()\np.";
    let (root, p) = temp_project(&[("main.luar", src)]);
    let main = root.join("main.luar");
    let names = labels_at(&p, &main, src, 2, 2);
    assert!(names.contains(&"name".to_string()), "{names:?}");
    assert!(names.contains(&"describe".to_string()));
}

#[test]
fn self_referential_alias_chains() {
    let src = "type varg = {\n    Input:(self:varg) -> varg,\n    count: number\n}\n\nlocal test:varg = nil\n\ntest:Input():";
    let (root, p) = temp_project(&[("main.luar", src)]);
    let main = root.join("main.luar");
    let line0 = src.lines().count() - 1;
    let col = src.lines().last().unwrap().chars().count();
    let names = labels_at(&p, &main, src, line0, col);
    assert!(
        names.contains(&"Input".to_string()),
        "chained self-type members missing: {names:?}"
    );
    let dot = format!("{}test:Input().", src.trim_end_matches("test:Input():"));
    let (root2, p2) = temp_project(&[("main.luar", &dot)]);
    let main2 = root2.join("main.luar");
    let line0 = dot.lines().count() - 1;
    let col = dot.lines().last().unwrap().chars().count();
    let names = labels_at(&p2, &main2, &dot, line0, col);
    assert!(names.contains(&"count".to_string()), "{names:?}");
}

#[test]
fn table_type_metamethods_drive_operators() {
    let src = "type Vec = {\n    __add:(self:Vec, other:Vec) -> Vec,\n    __sub:(self:Vec, other:number) -> number,\n    x: number\n}\nlocal a: Vec = make()\nlocal b: Vec = make()\nlocal sum = a + b\nlocal diff = a - 5\nlocal sx = sum.x\n";
    let (root, p) = temp_project(&[("main.luar", src)]);
    let main = root.join("main.luar");
    let info = p.file(&main).unwrap();
    assert_eq!(
        info.analysis.type_of("diff"),
        Some(&Type::Number),
        "__sub output not used"
    );
    assert_eq!(info.analysis.type_of("sx"), Some(&Type::Number), "__add output not chained");
}
