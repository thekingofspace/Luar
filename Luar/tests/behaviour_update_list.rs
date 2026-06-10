use luar::{Interpreter, NativeClassBuilder, Value};
use std::cell::RefCell;
use std::rc::Rc;

#[test]
fn behaviours_collected_and_ticked_directly_from_rust() {
    let updatables: Rc<RefCell<Vec<Value>>> = Rc::new(RefCell::new(Vec::new()));

    let mut host = Interpreter::new();
    host.define_class(NativeClassBuilder::new("Behaviour").make_abstract());

    let list = updatables.clone();
    host.on_instance_of("Behaviour", move |i, instance| {
        if i.instance_has_method(&instance, "update") {
            list.borrow_mut().push(instance);
        }
    });

    host.run_source(
        r#"
        class Mover extends Behaviour {
            public x: number = 0
            constructor(startX) self.x = startX end
            function update() self.x = self.x + 1; print("ran") end
        }
        class Prop extends Behaviour {
            public label: string = "static"
        }
        pub local keep = { Mover(10), Mover(100), Prop() }
        "#,
    )
    .expect("luar program should run");

    assert_eq!(updatables.borrow().len(), 2, "only the two Movers have update()");

    for _tick in 0..3 {
        let frame: Vec<Value> = updatables.borrow().clone();
        for inst in &frame {
            host.call_method(inst, "update", Vec::new()).expect("update should run");
        }
    }

    let xs: Vec<i64> = updatables
        .borrow()
        .iter()
        .map(|inst| inst.field(&Value::str("x")).as_int().unwrap_or(0))
        .collect();
    assert_eq!(xs, vec![13, 103]);
}

#[test]
fn instance_has_method_reports_overrides_and_absence() {
    let mut host = Interpreter::new();
    host.run_source(
        r#"
        class Base {}
        class WithTick extends Base { function tick() return 1 end }
        pub local a = WithTick()
        pub local b = Base()
        "#,
    )
    .unwrap();

    let a = host.get_global("a").unwrap();
    let b = host.get_global("b").unwrap();
    assert!(host.instance_has_method(&a, "tick"));
    assert!(!host.instance_has_method(&b, "tick"));
    assert!(!host.instance_has_method(&Value::Int(3), "tick"));
}

#[test]
fn ticking_a_freed_behaviours_method_errors() {
    let mut host = Interpreter::new();
    let id = host.current_script_id();
    host.run_source(
        r#"
        class Mover { public x: number = 0  function update() self.x = self.x + 1 end }
        pub local m = Mover()
        "#,
    )
    .unwrap();

    let m = host.get_global("m").unwrap();
    host.call_method(&m, "update", Vec::new()).expect("update works before free");

    host.free_script(id);

    assert!(host.call_method(&m, "update", Vec::new()).is_err());
}
