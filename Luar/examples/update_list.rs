use luar::{Interpreter, Value};
use std::cell::RefCell;
use std::rc::Rc;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let updatables: Rc<RefCell<Vec<Value>>> = Rc::new(RefCell::new(Vec::new()));

    let mut engine = Interpreter::new();
    engine.run_source("class MonoBehaviour {}")?;

    {
        let list = updatables.clone();
        engine.on_instance_of("MonoBehaviour", move |i, inst| {
            if i.instance_has_method(&inst, "Update") {
                list.borrow_mut().push(inst);
            }
        });
    }

    engine.run_source(
        r#"class Spinner extends MonoBehaviour {
               public angle: number = 0
               function Update() self.angle = self.angle + 10 end
           }
           class Mover extends MonoBehaviour {
               public x: number = 0
               function Update() self.x = self.x + 1 end
           }
           class Static extends MonoBehaviour {
               public tag: string = "no update"
           }
           pub local keep = { Spinner(), Mover(), Spinner(), Static() }"#,
    )?;

    println!("auto-tracked {} updatable instances", updatables.borrow().len());

    for _frame in 0..3 {
        let snapshot: Vec<Value> = updatables.borrow().clone();
        for inst in &snapshot {
            engine.call_method(inst, "Update", Vec::new())?;
        }
    }

    for (idx, inst) in updatables.borrow().iter().enumerate() {
        let angle = inst.field(&Value::str("angle"));
        let x = inst.field(&Value::str("x"));
        println!("  instance {idx}: angle={angle} x={x}");
    }

    Ok(())
}
