// Run with `npm test` (Node's own test runner; Node strips the types).
import { test } from "node:test";
import assert from "node:assert/strict";
import { DEFAULT_FILTERS, filtersToParams, parseFilters, withFilters, type IssueFilters } from "../src/lib/issueFilters.ts";

const url = (f: IssueFilters) => filtersToParams(f).toString();
const back = (s: string) => parseFilters(new URLSearchParams(s));

test("every filter survives a round trip through the URL", () => {
  const f: IssueFilters = { q: "#42", source: "github", group: "Dineout", project: "Laufey & Gustavo", agent: "a1", mine: true, closed: true };
  assert.deepEqual(back(url(f)), f);
  const words: IssueFilters = { ...DEFAULT_FILTERS, q: "retry cap @octo-dev", source: "loft" };
  assert.deepEqual(back(url(words)), words);
});

test("filters at their default are left out of the URL", () => {
  assert.equal(url(DEFAULT_FILTERS), "");
  assert.equal(url({ ...DEFAULT_FILTERS, source: "github" }), "source=github");
  assert.equal(url({ ...DEFAULT_FILTERS, q: "   " }), "");
  assert.equal(url({ ...DEFAULT_FILTERS, project: "api", mine: true }), "project=api&mine=1");
  assert.equal(url({ ...DEFAULT_FILTERS, group: "Dineout", project: "dineout/api" }), "group=Dineout&project=dineout%2Fapi");
  assert.deepEqual(back(""), DEFAULT_FILTERS);
});

test("unknown and invalid values fall back to the default", () => {
  assert.deepEqual(back("source=gitlab&mine=yes&closed=true&page=3&q=" + "x".repeat(201)), DEFAULT_FILTERS);
  assert.equal(back("source=GITHUB").source, "all");
  assert.equal(back("q=%20%23 42 ").q, "# 42");
});

test("the old Issues tab's links still land on the right filters", () => {
  assert.deepEqual(back("source=github"), { ...DEFAULT_FILTERS, source: "github" });
  assert.deepEqual(back("q=%2361&project=api"), { ...DEFAULT_FILTERS, q: "#61", project: "api" });
});

test("a filter changed in one view is still set in the other, and the view's own params stay", () => {
  const list = new URLSearchParams("project=api&agent=a1&q=retry");
  const specs = withFilters(withView(list), { mine: true });
  assert.deepEqual(back(specs.toString()), { ...DEFAULT_FILTERS, project: "api", agent: "a1", q: "retry", mine: true });
  // and back to the list: the same filters
  specs.delete("view");
  assert.deepEqual(back(specs.toString()), back("project=api&agent=a1&q=retry&mine=1"));
  // board options and the open spec survive a filter change
  const board = withFilters(new URLSearchParams("view=specs&stage=planned&needsme=1&open=t1&source=loft"), { source: "all", group: "Dineout" });
  assert.equal(board.get("view"), "specs");
  assert.equal(board.get("stage"), "planned");
  assert.equal(board.get("needsme"), "1");
  assert.equal(board.get("open"), "t1");
  assert.equal(board.get("source"), null);
  assert.equal(board.get("group"), "Dineout");
});

function withView(p: URLSearchParams) { const n = new URLSearchParams(p); n.set("view", "specs"); return n; }
