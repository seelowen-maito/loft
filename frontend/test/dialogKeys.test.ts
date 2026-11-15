import { test } from "node:test";
import assert from "node:assert/strict";
import { closesOnEscape } from "../src/lib/dialogKeys.ts";

const press = (o: Record<string, unknown> = {}) => ({ key: "Escape", defaultPrevented: false, target: { tagName: "BUTTON" }, ...o }) as Parameters<typeof closesOnEscape>[0];

test("Esc closes a dialog", () => assert.equal(closesOnEscape(press()), true));
test("other keys do not", () => assert.equal(closesOnEscape(press({ key: "Enter" })), false));
test("Esc in a field or already handled does not", () => {
  assert.equal(closesOnEscape(press({ target: { tagName: "TEXTAREA" } })), false);
  assert.equal(closesOnEscape(press({ target: { tagName: "DIV", isContentEditable: true } })), false);
  assert.equal(closesOnEscape(press({ defaultPrevented: true })), false);
});
