import { test } from "node:test";
import assert from "node:assert/strict";
import { MAC_ONLY, offered } from "../src/lib/platform.ts";
import { fixedShortcuts, shortcutAction } from "../src/lib/shortcuts.ts";

// What /api/info answers on each system (the rest of its features trimmed).
const mac = { platform: "macos", features: ["tasks", "reply", "simulator", "voice", "mac_access", "terminal_login", "herdr"] };
const linux = { platform: "linux", features: ["tasks", "reply"] };

test("on a Mac every Mac-only feature shows", () => {
  assert.deepEqual(offered(mac), { simulator: true, voice: true, mac_access: true, terminal_login: true, herdr: true });
});

test("off the Mac none of them shows", () => {
  for (const f of MAC_ONLY) assert.equal(offered(linux)[f], false, f);
  assert.equal(offered({ platform: "windows", features: ["tasks"] }).voice, false);
});

test("before /api/info answers, or from a server that lists no features, none shows", () => {
  assert.equal(Object.values(offered(null)).some(Boolean), false);
  assert.equal(Object.values(offered({})).some(Boolean), false);
});

test("each feature is read on its own", () => {
  const o = offered({ features: ["voice"] });
  assert.equal(o.voice, true);
  assert.equal(o.simulator, false);
});

const combos = { switcher: "Mod+K", next: "Mod+J" };
const key = (k: string) => ({ key: k, code: `Key${k.toUpperCase()}`, ctrlKey: false, altKey: false, shiftKey: false, metaKey: false });

test("g m goes to the Simulator only where the server offers it", () => {
  const on = offered(mac).simulator;
  const off = offered(linux).simulator;
  assert.deepEqual(shortcutAction(key("m"), { combos, typing: false, afterG: true, simulator: on }), { type: "go", path: "/simulator" });
  assert.equal(shortcutAction(key("m"), { combos, typing: false, afterG: true, simulator: off }), null);
  assert.deepEqual(shortcutAction(key("r"), { combos, typing: false, afterG: true, simulator: off }), { type: "go", path: "/repos" });
});

test("the shortcut list names the Simulator only where it's offered", () => {
  assert.ok(fixedShortcuts(offered(mac).simulator).some((s) => s.label === "Go to Simulator"));
  assert.ok(!fixedShortcuts(offered(linux).simulator).some((s) => s.label === "Go to Simulator"));
  assert.ok(fixedShortcuts(false).some((s) => s.label === "Go to Settings"));
});
