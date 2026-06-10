use luar::Value;

const FULL_FEATURE_SOURCE: &str = r#"
pub local log = ""

type Id = number
local typed: Id = 7
local cast = typed :: number

enum Color { Red Green = 10 Blue }

interface Named { name }

class Animal implements Named {
    public name: string = "animal"
    static count: number = 0

    constructor(n)
        self.name = n
        Animal.count = Animal.count + 1
    end

    function speak(): string
        return "..."
    end

    get label() return "<" .. self.name .. ">" end
    set label(v) self.name = v end

    operator +(o)
        return Animal(self.name .. o.name)
    end
}

class Loud {
    public volume: number = 11
}

class Dog extends Animal mixin Loud {
    override function speak(): string
        return "woof"
    end
}

local d = Dog("rex")
log = log .. d:speak() .. "|" .. d.label .. "|" .. d.volume .. "|"

local merged = Animal("a") + Animal("b")
log = log .. merged.name .. "|"

local function varargs(...)
    local t = {...}
    return #t
end
log = log .. varargs(1, 2, 3) .. "|"

local function fib(n)
    if n < 2 then
        return n
    end
    return fib(n - 1) + fib(n - 2)
end
log = log .. fib(10) .. "|"

local acc = 0
for i = 1, 5 do
    acc += i
end
while acc > 12 do
    acc -= 1
end
log = log .. acc .. "|"

local arr = { 1, 2, 3, extra = "x" }
local sum = 0
for i, v in ipairs(arr) do
    sum = sum + v
end
for k, v in pairs({ only = 4 }) do
    sum = sum + v
end
log = log .. sum .. "|"

local word = switch(Color.Green)
    case 10
        return "ten"
    end
    default
        return "other"
    end
end
log = log .. word .. "|"

local interp = `v={typed}`
log = log .. interp .. "|"

local co = coroutine.create(function(x)
    local got = coroutine.yield(x + 1)
    return got * 2
end)
local ok1, first = coroutine.resume(co, 41)
local ok2, second = coroutine.resume(co, 5)
log = log .. first .. "," .. second .. "|"

buff 64 scratch = "small"
log = log .. scratch .. "|"
freebuff scratch

local maybe = nil or "fallback"
log = log .. maybe .. "|"
log = log .. (true and 1 or 2) .. "|"
log = log .. tostring(not false)
"#;

#[test]
fn precompile_roundtrip_covers_every_feature() {
    let direct = luar::eval_source(FULL_FEATURE_SOURCE).expect("direct run");
    let direct_log = direct.get_global("log").expect("direct log");

    let bytes = luar::precompile_source(FULL_FEATURE_SOURCE).expect("precompile");
    let packed = luar::run_precompiled(&bytes).expect("run precompiled");
    let packed_log = packed.get_global("log").expect("packed log");

    assert_eq!(direct_log, packed_log, "precompiled run diverged from direct run");
    match &packed_log {
        Value::Str(s) => {
            assert!(s.contains("woof"), "{s}");
            assert!(s.contains("ten"), "{s}");
            assert!(s.contains("42,10"), "{s}");
        }
        other => panic!("expected string log, got {other:?}"),
    }
}

#[test]
fn precompile_roundtrip_preserves_module_returns() {
    let src = "local M = { value = 5 }\nreturn M";
    let bytes = luar::precompile_source(src).expect("precompile");
    let value = luar::load_precompiled_module(&bytes).expect("load module");
    match value {
        Value::Table(t) => {
            let v = t.borrow().get(&Value::str("value"));
            assert_eq!(v, Value::Int(5));
        }
        other => panic!("expected table, got {other:?}"),
    }
}
