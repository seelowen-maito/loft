import { test } from "node:test";
import assert from "node:assert/strict";
import { dayLabel, fmtClock, fmtDate, fmtDateTime, fmtDayTime, fmtDuration, fmtMonth, nextRunText, relTime } from "../src/lib/time.ts";

const now = new Date(2026, 9, 27, 12, 0).getTime(); // Tue 12:00 local
const ago = (s: number) => new Date(now - s * 1000).toISOString();

test("relTime words the past and the future the same way", () => {
  assert.equal(relTime(ago(10), now), "just now");
  assert.equal(relTime(ago(-10), now), "just now");
  assert.equal(relTime(ago(300), now), "5m ago");
  assert.equal(relTime(ago(-7200), now), "in 2h");
  assert.equal(relTime(ago(3 * 86400), now), "3d ago");
  assert.equal(relTime(ago(-3 * 86400), now), "in 3d");
  assert.equal(relTime(ago(90 * 86400), now), "3mo ago");
  assert.equal(relTime(ago(800 * 86400), now), "2y ago");
});

test("relTime takes epoch seconds, and says never for no time", () => {
  assert.equal(relTime(now / 1000 - 600, now), "10m ago");
  assert.equal(relTime(null, now), "never");
  assert.equal(relTime(undefined, now), "never");
  assert.equal(relTime("", now), "never");
});

test("a check-in due, overdue or under 45 s away is any moment; a later one says when", () => {
  assert.equal(nextRunText(ago(300), now), "next: any moment", "overdue");
  assert.equal(nextRunText(ago(0), now), "next: any moment", "due");
  assert.equal(nextRunText(ago(-30), now), "next: any moment");
  assert.equal(nextRunText(ago(-300), now), "next in 5m");
});

test("fmtDuration reads seconds, minutes, hours, then days", () => {
  assert.equal(fmtDuration(0), "0s");
  assert.equal(fmtDuration(45_900), "45s");
  assert.equal(fmtDuration(60_000), "1m 0s");
  assert.equal(fmtDuration(143_000), "2m 23s");
  assert.equal(fmtDuration(60 * 60_000), "1h 0m");
  assert.equal(fmtDuration(3_849_000), "1h 4m");
  assert.equal(fmtDuration(125 * 60_000), "2h 5m");
  assert.equal(fmtDuration((2 * 86400 + 3 * 3600 + 59) * 1000), "2d 3h");
  assert.equal(fmtDuration(-5_000), "0s", "clock skew");
});

test("clock times follow the locale's hour cycle", () => {
  const t = new Date(2026, 9, 27, 14, 5, 9);
  assert.equal(fmtClock(t, { locale: "en-GB" }), "14:05");
  assert.equal(fmtClock(t, { locale: "en-GB", seconds: true }), "14:05:09");
  assert.match(fmtClock(t, { locale: "en-US" }), /^2:05\sPM$/);
  assert.match(fmtClock(t, { locale: "en-US", seconds: true }), /^2:05:09\sPM$/);
  assert.equal(fmtClock(t.getTime() / 1000, { locale: "en-GB" }), "14:05", "epoch seconds");
});

test("dates and date-times are medium, in the locale", () => {
  const t = new Date(2026, 10, 15, 14, 5);
  assert.equal(fmtDate(t, "en-US"), "Nov 15, 2026");
  assert.equal(fmtDate(t.toISOString(), "en-GB"), "15 Nov 2026");
  assert.match(fmtDateTime(t, "en-US"), /^Nov 15, 2026,? 2:05\sPM$/);
  assert.match(fmtDateTime(t, "en-GB"), /^15 Nov 2026,? 14:05$/);
});

test("fmtDayTime says the weekday, and the date once it is over 6 days away", () => {
  assert.equal(fmtDayTime(new Date(2026, 9, 27, 14, 0), now, "en-GB"), "Tue 14:00");
  assert.match(fmtDayTime(new Date(2026, 9, 29, 9, 30), now, "en-US"), /^Thu 9:30\sAM$/);
  assert.match(fmtDayTime(new Date(2026, 10, 5, 9, 30), now, "en-US"), /^Thu,? Nov 5,? 9:30\sAM$/, "future, far");
  assert.match(fmtDayTime(new Date(2026, 9, 10, 9, 30), now, "en-US"), /Oct 10/, "past, far");
});

test("chart labels follow the locale", () => {
  assert.equal(fmtMonth(new Date(2026, 10, 15), "en-US"), "Nov");
  const d = new Date();
  d.setDate(d.getDate() - 2);
  assert.equal(dayLabel(2, "en-US"), d.toLocaleDateString("en-US", { weekday: "short", month: "short", day: "numeric" }));
});
