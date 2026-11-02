import test from "node:test";
import assert from "node:assert/strict";
import { syntaxLanguage, syntaxTokens, highlightDiff } from "../mods/annotator/syntax.js";

test("lexical colors preserve exact source including indentation, Unicode and markup", () => {
  for (const [path, text] of [
    ["file.ts", '  const emoji = "🦀 <script>\\\""; // comment'],
    ["file.rs", "pub fn fee(amount: u64) -> u64 { amount / 100 }"],
    ["file.py", "    if value is None: print('hello # world') # note"],
    ["file.sql", "SELECT id FROM users WHERE active = TRUE; -- note"],
    ["file.json", '  "answer": 42,'],
    ["file.yml", 'name: "value # data" # note'],
    ["file.sh", 'if [ "$value" = "ok" ]; then echo 1; fi # note'],
  ]) {
    const tokens = syntaxTokens(text, syntaxLanguage(path));
    assert.equal(tokens.map(t => t.text).join(""), text);
    assert.ok(tokens.some(t => t.kind !== "plain"), path);
  }
});

test("strings protect comment markers and JSON keys get their own token", () => {
  const ts = syntaxTokens('const url = "https://example.test"; // actual', "js");
  assert.equal(ts.filter(t => t.kind === "comment").map(t => t.text).join(""), "// actual");
  assert.ok(ts.some(t => t.kind === "keyword" && t.text === "const"));
  assert.ok(syntaxTokens('"name": "value"', "json").some(t => t.kind === "property" && t.text === '"name"'));
  assert.ok(syntaxTokens("fn get<'a>(v: &'a str) -> &'a str { v }", "rust").every(t => t.kind !== "string"));
});

test("old and new sides keep independent multiline states and reset at hidden gaps", () => {
  const rows = [
    { kind: "delete", old: 1, new: null, text: "/* removed opener" },
    { kind: "add", old: null, new: 1, text: "const fresh = 1;" },
    { kind: "context", old: 2, new: 2, text: "unchanged text" },
    { kind: "context", old: 10, new: 10, text: "return value;" },
  ];
  const tokens = highlightDiff(rows, "file.ts");
  assert.ok(tokens.get("old:2").every(t => t.kind === "comment"));
  assert.ok(tokens.get("new:1").some(t => t.kind === "keyword"));
  assert.ok(tokens.get("new:2").every(t => t.kind !== "comment"));
  assert.ok(tokens.get("old:10").some(t => t.kind === "keyword"));
});

test("multiline strings continue only across visible consecutive source lines", () => {
  const state = {};
  syntaxTokens('value = """first', "python", state);
  assert.equal(state.quote, '"""');
  const next = syntaxTokens('second""" # end', "python", state);
  assert.ok(next.some(t => t.kind === "comment"));
  assert.equal(state.quote, null);
});

test("unknown languages and extreme lines use bounded plain rendering", () => {
  assert.equal(syntaxLanguage("notes.txt"), null);
  assert.equal(highlightDiff([{ old: 1, new: 1, text: "plain" }], "notes.txt").size, 0);
  for (const text of ["x".repeat(9000), "const x = 1; ".repeat(300)]) {
    const tokens = syntaxTokens(text, "js");
    assert.equal(tokens.length, 1);
    assert.equal(tokens[0].kind, "plain");
    assert.equal(tokens[0].text, text);
  }
});
