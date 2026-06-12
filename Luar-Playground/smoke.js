const fs = require("fs");
const path = require("path");

const wasmPath = path.join(__dirname, "..", "docs", "luar.wasm");
const buf = fs.readFileSync(wasmPath);

function runCase(instance, source) {
  const { lp_alloc, lp_run, lp_dealloc, memory } = instance.exports;
  const src = new TextEncoder().encode(source);
  const ptr = lp_alloc(src.length);
  new Uint8Array(memory.buffer, ptr, src.length).set(src);
  const out = lp_run(ptr, src.length);
  const view = new DataView(memory.buffer);
  const len = view.getUint32(out, true);
  const status = view.getUint8(out + 4);
  const body = new TextDecoder().decode(
    new Uint8Array(memory.buffer, out + 5, len)
  );
  lp_dealloc(ptr, src.length);
  lp_dealloc(out, 5 + len);
  return { status, body };
}

WebAssembly.instantiate(buf, {
  env: { js_now_ms: () => Date.now() },
}).then(({ instance }) => {
  let failures = 0;
  const check = (name, source, wantStatus, wantIncludes) => {
    const { status, body } = runCase(instance, source);
    const ok = status === wantStatus && wantIncludes.every((w) => body.includes(w));
    console.log(`${ok ? "ok  " : "FAIL"} ${name} -> status=${status} body=${JSON.stringify(body)}`);
    if (!ok) failures++;
  };

  check("print", 'print("hello", 1 + 2)', 0, ["hello\t3"]);
  check(
    "class",
    'class P {\n    public Name:string = "sam"\n    public function Greet()\n        return `hi {self.Name}`\n    end\n}\nprint(P():Greet())',
    0,
    ["hi sam"]
  );
  check("table", "local t = { 3, 1, 2 }\ntable.sort(t)\nprint(table.concat(t, \",\"))", 0, ["1,2,3"]);
  check("clock", "print(os.clock() >= 0)", 0, ["true"]);
  check("switch", 'local w = switch(2)\n  case 1 return "a" end\n  case 2 return "b" end\nend\nprint(w)', 0, ["b"]);
  check("error", "local x = nil\nx()", 1, ["error:"]);
  check("coroutine gated", "coroutine.create(function() end)", 1, ["playground"]);
  check("tailcall", "local function loop(n) if n == 0 then return \"done\" end return loop(n - 1) end\nprint(loop(100000))", 0, ["done"]);

  process.exit(failures ? 1 : 0);
});
