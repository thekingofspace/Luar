const fs = require("fs");
const path = require("path");
const tm = require("vscode-textmate");
const oniguruma = require("vscode-oniguruma");

const wasmPath = path.join(
  __dirname,
  "..",
  "node_modules",
  "vscode-oniguruma",
  "release",
  "onig.wasm"
);

async function main() {
  const wasmBin = fs.readFileSync(wasmPath).buffer;
  const onigLib = oniguruma.loadWASM(wasmBin).then(() => ({
    createOnigScanner: (s) => new oniguruma.OnigScanner(s),
    createOnigString: (s) => new oniguruma.OnigString(s),
  }));
  const registry = new tm.Registry({
    onigLib,
    loadGrammar: async () =>
      tm.parseRawGrammar(
        fs.readFileSync(
          path.join(__dirname, "..", "syntaxes", "luar.tmLanguage.json"),
          "utf8"
        ),
        "luar.tmLanguage.json"
      ),
  });
  const grammar = await registry.loadGrammar("source.luar");

  const cases = [
    ["local function Get<i>(var)", "Get", "entity.name.function"],
    ["local function Get<i>(var)", "<", "keyword.control.generic"],
    ["local function Get<i>(var)", "i", "variable.other.property"],
    ["export type Varg = boolean", "boolean", "entity.name.type"],
    ["type Mode = \"on\" | \"off\"", "\"on\"", "entity.name.type"],
    ["type Mode = \"on\" | \"off\"", "|", "keyword.operator.type"],
    ["local test:boolean", "boolean", "entity.name.type"],
    ["local test: boolean = true", "boolean", "entity.name.type"],
    ["local mode: \"on\" | \"off\" = \"on\"", "\"off\"", "entity.name.type"],
    ["local m: Map<string, Array<number>> = x", "Map", "entity.name.type"],
    ["local m: Map<string, Array<number>> = x", "<", "keyword.control.generic"],
    ["local m: Map<string, Array<number>> = x", "string", "variable.other.property"],
    ["enum Grag { test, other }", "Grag", "entity.name.type.enum"],
    ["enum Grag { test, other }", "test", "variable.other.enummember"],
    ["local g: Test<any> = nil", "any", "variable.other.property"],
    ["function f(a: number, b: string): boolean", "number", "entity.name.type"],
    ["local v = x :: Shape?", "Shape", "entity.name.type"],
    ["local s: shapes.Shape = make()", "Shape", "entity.name.type"],
    ["if not true then", "if", "storage.modifier.control"],
    ["if not true then", "not", "storage.modifier.logical"],
    ["local x = grag.test", "test", "variable.other.property"],
    ["local t = { local = 1, type = 2 }", "type", "variable.other.property"],
    ["local m = p_data.Real[\"Money\"]", "Real", "variable.other.property"],
    ["local m = p_data.Real[idx]", "Real", "variable.other.property"],
  ];
  const negatives = [
    ["obj:method()", "method", "entity.name.type"],
    ["local t = { local = 1, type = 2 }", "type", "storage.type"],
    ["p:length()", "length", "entity.name.type"],
    ["obj:method \"sugar\"", "method", "entity.name.type"],
  ];

  let failures = 0;
  function tokensFor(line) {
    return grammar.tokenizeLine(line, tm.INITIAL).tokens.map((t) => ({
      text: line.slice(t.startIndex, t.endIndex),
      scopes: t.scopes.join(" "),
    }));
  }
  for (const [line, target, scope] of cases) {
    const toks = tokensFor(line);
    const covered = toks
      .filter((t) => t.scopes.includes(scope))
      .map((t) => t.text)
      .join("");
    if (!covered.replace(/\s+/g, "").includes(target.replace(/\s+/g, ""))) {
      failures++;
      console.log(`FAIL: "${line}" — expected "${target}" scoped ${scope}`);
      for (const t of toks) console.log("   ", JSON.stringify(t.text), "=>", t.scopes);
    } else {
      console.log(`ok:   "${line}"`);
    }
  }
  for (const [line, target, scope] of negatives) {
    const toks = tokensFor(line);
    const wrong = toks.find(
      (t) => t.text.includes(target) && t.scopes.includes(scope)
    );
    if (wrong) {
      failures++;
      console.log(`FAIL (negative): "${line}" — "${target}" wrongly scoped as type`);
    } else {
      console.log(`ok:   "${line}" (no false type)`);
    }
  }
  process.exit(failures ? 1 : 0);
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
