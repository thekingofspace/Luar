use luar::{Interpreter, NativeClassBuilder, Value};
use std::cell::RefCell;
use std::rc::Rc;

#[test]
fn rust_base_class_extended_in_luar_receives_update_ticks() {
    let spawned: Rc<RefCell<Vec<Value>>> = Rc::new(RefCell::new(Vec::new()));

    let mut host = Interpreter::new();

    host.define_class(NativeClassBuilder::new("Behaviour").make_abstract());

    let collected = spawned.clone();
    host.on_instance_of("Behaviour", move |_i, instance| {
        collected.borrow_mut().push(instance);
    });

    host.on_subclass_of("Behaviour", |i, class| {
        let name = match i.get_global("classname") {
            Some(f) => i
                .call_value(&f, vec![class.clone()])
                .ok()
                .and_then(|r| r.into_iter().next())
                .map(|v| v.to_string())
                .unwrap_or_default(),
            None => String::new(),
        };
        println!("a class extended Behaviour: {name} (its instances will be ticked)");
    });

    host.run_source(
        r#"
        pub function runTick(obj)
            for _, name in ipairs(methodsof(obj)) do
                if name == "update" then
                    obj:update()
                    return true
                end
            end
            return false
        end

        class Mover extends Behaviour {
            public x: number = 0
            constructor(startX) self.x = startX end
            function update() self.x = self.x + 1; print("test") end
        }

        pub local a = Mover(10)
        pub local b = Mover(100)
        "#,
    )
    .expect("luar program should run");

    let run_tick = host.get_global("runTick").expect("runTick should be defined");

    for tick in 1..=3 {
        let frame: Vec<Value> = spawned.borrow().clone();
        println!("tick {tick}: updating {} behaviour(s)", frame.len());
        for inst in frame {
            host.call_value(&run_tick, vec![inst]).expect("update tick should run");
        }
    }

    assert_eq!(spawned.borrow().len(), 2, "both Movers collected at construction");

    host.run_source("pub local ax = a.x\npub local bx = b.x").expect("read back fields");
    assert_eq!(host.get_global("ax").and_then(|v| v.as_int()), Some(13));
    assert_eq!(host.get_global("bx").and_then(|v| v.as_int()), Some(103));
}
