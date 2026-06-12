const fs = require("fs");
const html = fs.readFileSync(__dirname + "/playground.html", "utf8");
const script = html.match(/<script>([\s\S]*)<\/script>/)[1];

const head = script
  .split("const editor =")[0]
  .concat(
    script.match(/const KEYWORDS[\s\S]*?const TYPES[\s\S]*?\]\);/)[0],
    script.match(/function escapeHtml[\s\S]*?\n}/)[0],
    script.match(/const TOKEN_RE = .*;/)[0],
    script.match(/function highlightBacktick[\s\S]*?\nfunction refreshHighlight/)[0].replace(/\nfunction refreshHighlight$/, "")
  );

const sandbox = new Function(
  head +
    `
const sample = 'local x = "hi there" -- note\\nprint(\`val {self.Name} says woof!\`, 42)\\nclass Dog extends Animal {}\\n--[[ block\\ncomment ]]\\nlocal s: string = "y"';
return highlightSource(sample);
`
);
const out = sandbox();
const checks = [
  ['<span class="kw">local</span>', "keyword"],
  ['<span class="str">"hi there"</span>', "string"],
  ['<span class="com">-- note</span>', "line comment"],
  ['<span class="fn">print</span>', "function name"],
  ['<span class="num">42</span>', "number"],
  ['<span class="kw">class</span>', "class keyword"],
  ['<span class="com">--[[ block\ncomment ]]</span>', "block comment"],
  ['<span class="typ">string</span>', "type name"],
  ['<span class="str">`val {</span>', "backtick opens as string"],
  ['<span class="kw">self</span>', "interpolation expr highlighted"],
  ['<span class="str"> says woof!`</span>', "backtick tail as string"],
];
let failures = 0;
for (const [needle, label] of checks) {
  const ok = out.includes(needle);
  console.log(`${ok ? "ok  " : "FAIL"} ${label}`);
  if (!ok) failures++;
}
if (failures) {
  console.log(out);
  process.exit(1);
}
