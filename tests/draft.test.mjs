import test from "node:test";
import assert from "node:assert/strict";
import { DraftLifecycle } from "../mods/annotator/draft.js";

function fixture() {
  let text = "";
  const editor = { value: () => text, set_value: value => { text = value; } };
  return { editor, draft: new DraftLifecycle(editor) };
}

test("invalidated intent survives repeated checkout round trips", () => {
  const { editor, draft } = fixture();
  draft.observe("r1"); draft.anchor({ side: "new", start: 2, end: 2 }, "r1");
  editor.set_value("line intent");
  assert.equal(draft.observe("r2", "r1"), true);
  for (let i = 0; i < 3; i++) {
    draft.enter("other", "checkout", { open: "a.rs" }, "r2");
    draft.enter("checkout", "other", {}, "other");
    assert.equal(draft.observe("r2"), true);
    assert.equal(draft.revision, "r1");
    assert.equal(editor.value(), "line intent");
  }
  draft.anchor(null, "r2");
  assert.equal(draft.observe("r2"), false);
});

test("Add completion never clears a later edit or another checkout", () => {
  const { editor, draft } = fixture();
  draft.observe("r1"); editor.set_value("first");
  const token = draft.capture("checkout");
  editor.set_value("later");
  assert.equal(draft.complete(token, "checkout"), false);
  assert.equal(editor.value(), "later");
  draft.enter("other", "checkout", {}, "r1"); editor.set_value("unrelated");
  draft.complete(token, "other");
  assert.equal(editor.value(), "unrelated");
  assert.equal(draft.saved.get("checkout").body, "later");
});
