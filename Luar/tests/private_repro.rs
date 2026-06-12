use luar::{eval_source, Value};

#[test]
fn private_field_with_table_clear_in_method() {
    let r = eval_source(
        r#"class S {
    private Items:{number} = {}
    public function Reset()
        table.insert(self.Items, 1)
        table.insert(self.Items, 2)
        table.clear(self.Items)
        return #self.Items
    end
}
pub local n = S():Reset()"#,
    );
    match r {
        Ok(i) => assert_eq!(i.env.get("n"), Some(Value::Int(0))),
        Err(e) => panic!("method+table.clear failed: {e}"),
    }
}

#[test]
fn private_field_read_in_method() {
    let r = eval_source(
        r#"class S {
    private Secret:number = 7
    public function Get()
        return self.Secret
    end
}
pub local n = S():Get()"#,
    );
    match r {
        Ok(i) => assert_eq!(i.env.get("n"), Some(Value::Int(7))),
        Err(e) => panic!("private read failed: {e}"),
    }
}

#[test]
fn private_field_in_closure_made_inside_method() {
    let r = eval_source(
        r#"class S {
    private Items:{number} = {}
    public function Maker()
        return function()
            table.insert(self.Items, 1)
            return #self.Items
        end
    end
}
local fn = S():Maker()
pub local n = fn()"#,
    );
    match r {
        Ok(i) => assert_eq!(i.env.get("n"), Some(Value::Int(1))),
        Err(e) => panic!("closure private access failed: {e}"),
    }
}

#[test]
fn closure_from_method_can_write_private_fields() {
    let r = eval_source(
        r#"class S {
    private Count:number = 0
    public function Bumper()
        return function()
            self.Count = self.Count + 1
            return self.Count
        end
    end
}
local bump = S():Bumper()
bump()
bump()
pub local n = bump()"#,
    );
    match r {
        Ok(i) => assert_eq!(i.env.get("n"), Some(Value::Int(3))),
        Err(e) => panic!("closure private write failed: {e}"),
    }
}

#[test]
fn closures_made_outside_the_class_are_still_denied() {
    let r = eval_source(
        r#"class S {
    private Secret:number = 7
}
local s = S()
local peek = function()
    return s.Secret
end
pub local v = peek()"#,
    );
    let err = r.err().expect("outside closure should be denied");
    assert!(err.to_string().contains("private"), "{err}");
}

#[test]
fn table_clear_preserves_table_identity() {
    let r = eval_source(
        r#"local t = { 1, 2, 3 }
local alias = t
table.clear(t)
alias[1] = 99
pub local same = rawequal(t, alias)
pub local v = t[1]"#,
    );
    match r {
        Ok(i) => {
            assert_eq!(i.env.get("same"), Some(Value::Bool(true)));
            assert_eq!(i.env.get("v"), Some(Value::Int(99)));
        }
        Err(e) => panic!("identity check failed: {e}"),
    }
}

#[test]
fn private_field_in_method_run_as_coroutine() {
    let r = eval_source(
        r#"class S {
    private Items:{number} = {}
    public function Fill()
        table.insert(self.Items, 1)
        table.clear(self.Items)
        return #self.Items
    end
}
local s = S()
local co = coroutine.create(function()
    return s:Fill()
end)
pub local ok, n = coroutine.resume(co)"#,
    );
    match r {
        Ok(i) => {
            assert_eq!(i.env.get("ok"), Some(Value::Bool(true)));
            assert_eq!(i.env.get("n"), Some(Value::Int(0)));
        }
        Err(e) => panic!("coroutine method failed: {e}"),
    }
}
