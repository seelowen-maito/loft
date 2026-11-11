import { test } from "node:test";
import assert from "node:assert/strict";
import { herdrInstallCommand, isLocalOrigin } from "../src/lib/herdr.ts";

test("the install command carries the Loft address twice: to fetch the script and to point it there", () => {
  assert.equal(
    herdrInstallCommand("https://homebase.example.ts.net/"),
    "curl -fsSL https://homebase.example.ts.net/api/herdr/install.sh | sh -s -- https://homebase.example.ts.net",
  );
});

test("localhost addresses are flagged: other machines can't use them", () => {
  assert.equal(isLocalOrigin("http://localhost:7421"), true);
  assert.equal(isLocalOrigin("http://127.0.0.1:7420"), true);
  assert.equal(isLocalOrigin("https://homebase.example.ts.net"), false);
  assert.equal(isLocalOrigin("http://192.168.1.20:7420"), false);
});
