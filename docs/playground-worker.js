let instance = null;

const loaded = WebAssembly.instantiateStreaming(
  fetch("luar.wasm"),
  { env: { js_now_ms: () => performance.now() } }
).catch(() =>
  fetch("luar.wasm")
    .then((r) => r.arrayBuffer())
    .then((buf) =>
      WebAssembly.instantiate(buf, {
        env: { js_now_ms: () => performance.now() },
      })
    )
);

loaded.then((result) => {
  instance = result.instance;
  postMessage({ kind: "ready" });
});

function runSource(source) {
  const { lp_alloc, lp_dealloc, lp_run, memory } = instance.exports;
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

onmessage = (e) => {
  if (e.data.kind !== "run" || !instance) {
    return;
  }
  const started = performance.now();
  let result;
  try {
    result = runSource(e.data.source);
  } catch (err) {
    result = { status: 1, body: "playground error: " + err.message };
  }
  postMessage({
    kind: "result",
    status: result.status,
    body: result.body,
    ms: performance.now() - started,
  });
};
