import assert from "node:assert/strict";
import test from "node:test";
import {
  describeQueryRange,
  previousPeriodPlan,
  queryKind,
  resolveQueryPlan,
  retentionModel,
  seriesValues,
} from "../src/analytics-model.ts";
import type { QueryResult } from "../src/types.ts";

const DAY_NANOS = 86_400_000_000_000;
const NOW_MS = Date.UTC(2026, 6, 24, 12);

test("relative saved-query ranges resolve at execution time without mutating the saved plan", () => {
  const plan = {
    version: 1,
    kind: "aggregate",
    range: {
      start_unix_nano: 1,
      end_unix_nano: 2,
      relative: { amount: 7, unit: "day" },
    },
    aggregate: {},
  };
  const resolved = resolveQueryPlan(plan, NOW_MS);
  const range = resolved.range as Record<string, unknown>;
  assert.equal(range.end_unix_nano, NOW_MS * 1_000_000);
  assert.equal(range.start_unix_nano, NOW_MS * 1_000_000 - 7 * DAY_NANOS);
  assert.deepEqual((plan.range as Record<string, unknown>).start_unix_nano, 1);
  assert.match(describeQueryRange(plan, NOW_MS), /^Rolling 7 days ·/);
});

test("previous-period comparisons use an adjacent evaluated range and remove refresh metadata", () => {
  const plan = {
    version: 1,
    kind: "events",
    range: {
      start_unix_nano: 1,
      end_unix_nano: 2,
      relative: { amount: 7, unit: "day" },
    },
    events: {},
  };
  const previous = previousPeriodPlan(plan, NOW_MS);
  const range = previous.range as Record<string, unknown>;
  assert.equal(range.end_unix_nano, NOW_MS * 1_000_000 - 7 * DAY_NANOS);
  assert.equal(range.start_unix_nano, NOW_MS * 1_000_000 - 14 * DAY_NANOS);
  assert.equal(range.relative, undefined);
});

test("query kinds stay explicit so results can be isolated across builders", () => {
  assert.equal(queryKind({ kind: "funnel" }), "funnel");
  assert.equal(queryKind({ kind: "unknown" }), null);
  assert.equal(queryKind({}), null);
});

test("retention output discards invalid periods instead of rendering PNaN", () => {
  const result: QueryResult = {
    columns: ["cohort_bucket", "period", "cohort_subjects", "retention_rate"],
    rows: [
      ["2026-07-20", undefined, 10, 0.5],
      ["2026-07-20", 1, 10, 0.4],
      ["2026-07-21", 0, 5, Number.NaN],
    ],
    stats: {
      cache_hit: false,
      file_count: 1,
      scan_bytes: 12,
      row_count: 3,
      total_duration_nano: 1,
    },
  };
  const model = retentionModel(result);
  assert.deepEqual(model.periods, [1]);
  assert.equal(model.cohorts.length, 1);
  assert.equal(model.cohorts[0]?.values.get(1), 0.4);
});

test("series output includes only finite values and a readable label", () => {
  const result: QueryResult = {
    columns: ["bucket", "metric_value"],
    rows: [["Jul 23", 12], ["Jul 24", Number.NaN], ["Jul 25", 18]],
    stats: {
      cache_hit: false,
      file_count: 1,
      scan_bytes: 12,
      row_count: 3,
      total_duration_nano: 1,
    },
  };
  assert.deepEqual(seriesValues(result), [
    { label: "Jul 23", value: 12 },
    { label: "Jul 25", value: 18 },
  ]);
});
