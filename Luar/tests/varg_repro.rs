#[test]
fn varargs_forward_through_method_to_resume() {
    let interp = luar::eval_source(
        r#"pub local log = ""
local function h(a, b)
    log = tostring(a) .. "," .. tostring(b)
end
class S {
    function fire(...)
        coroutine.resume(coroutine.create(h), ...)
    end
}
S():fire("x", 10)"#,
    )
    .unwrap();
    luar::run_pending();
    assert_eq!(interp.get_global("log"), Some(luar::Value::str("x,10")));
}
