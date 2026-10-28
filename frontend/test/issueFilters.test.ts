// Run with `npm test` (Node's own test runner; Node strips the types).
import { test } from "node:test";
import assert from "node:assert/strict";
import { DEFAULT_FILTERS, filtersToParams, parseFilters, type IssueFilters } from "../src/lib/issueFilters.ts";

const url = (f: IssueFilters) => filtersToParams(f).toString();
const back = (s: string) => parseFilters(new URLSearchParams(s));

test("every filter survives a round trip through the URL", () => {
  const f: IssueFilters = { q: "#42", source: "github", project: "Laufey & Gustavo", agent: "a1", mine: true, closed: true };
  assert.deepEqual(back(url(f)), f);
  const words: IssueFilters = { ...DEFAULT_FILTERS, q: "retry cap @octo-dev", source: "loft" };
  assert.deepEqual(back(url(words)), words);
});

test("filters at their default are left out of the URL", () => {
  assert.equal(url(DEFAULT_FILTERS), "");
  assert.equal(url({ ...DEFAULT_FILTERS, source: "github" }), "source=github");
  assert.equal(url({ ...DEFAULT_FILTERS, q: "   " }), "");
  assert.equal(url({ ...DEFAULT_FILTERS, project: "api", mine: true }), "project=api&mine=1");
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
