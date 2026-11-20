import { test } from "node:test";
import assert from "node:assert/strict";
import { closesOnEscape, fromField } from "../src/lib/dialogKeys.ts";

const press = (o: Record<string, unknown> = {}) => ({ key: "Escape", defaultPrevented: false, ...o }) as Parameters<typeof closesOnEscape>[0];

test("Esc closes a dialog", () => assert.equal(closesOnEscape(press()), true));
test("other keys do not", () => assert.equal(closesOnEscape(press({ key: "Enter" })), false));
test("Esc already handled does not", () => assert.equal(closesOnEscape(press({ defaultPrevented: true })), false));
test("fields are inputs, textareas, selects and contenteditables", () => {
  assert.equal(fromField({ tagName: "TEXTAREA" } as unknown as EventTarget), true);
  assert.equal(fromField({ tagName: "DIV", isContentEditable: true } as unknown as EventTarget), true);
  assert.equal(fromField({ tagName: "BUTTON" } as unknown as EventTarget), false);
  assert.equal(fromField(null), false);
});
