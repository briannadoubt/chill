import type { QueryResult } from "./types";

export type AnalyticsKind =
  | "events"
  | "trace"
  | "replay"
  | "funnel"
  | "cohort"
  | "aggregate"
  | "path"
  | "retention";

export type RelativeRange = {
  amount: number;
  unit: "hour" | "day" | "week";
};

export type EvaluatedRange = {
  startUnixNano: number;
  endUnixNano: number;
  relative: RelativeRange | null;
};

const NANOSECONDS_PER_MILLISECOND = 1_000_000;
const MILLISECONDS_PER_UNIT: Record<RelativeRange["unit"], number> = {
  hour: 60 * 60 * 1_000,
  day: 24 * 60 * 60 * 1_000,
  week: 7 * 24 * 60 * 60 * 1_000,
};

export function queryKind(plan: Record<string, unknown>): AnalyticsKind | null {
  const kind = plan.kind;
  return typeof kind === "string" && [
    "events",
    "trace",
    "replay",
    "funnel",
    "cohort",
    "aggregate",
    "path",
    "retention",
  ].includes(kind)
    ? kind as AnalyticsKind
    : null;
}

export function resolveQueryPlan(
  plan: Record<string, unknown>,
  nowMilliseconds = Date.now(),
): Record<string, unknown> {
  const copy = structuredClone(plan);
  const range = rangeObject(copy);
  const relative = parseRelativeRange(range?.relative);
  if (!range || !relative) return copy;
  const endUnixNano = Math.floor(nowMilliseconds) * NANOSECONDS_PER_MILLISECOND;
  const duration = relative.amount * MILLISECONDS_PER_UNIT[relative.unit] * NANOSECONDS_PER_MILLISECOND;
  range.start_unix_nano = endUnixNano - duration;
  range.end_unix_nano = endUnixNano;
  return copy;
}

export function previousPeriodPlan(
  plan: Record<string, unknown>,
  nowMilliseconds = Date.now(),
): Record<string, unknown> {
  const copy = resolveQueryPlan(plan, nowMilliseconds);
  const range = rangeObject(copy);
  const start = finiteNumber(range?.start_unix_nano);
  const end = finiteNumber(range?.end_unix_nano);
  if (!range || start === null || end === null || start >= end) return copy;
  const duration = end - start;
  range.start_unix_nano = start - duration;
  range.end_unix_nano = end - duration;
  delete range.relative;
  return copy;
}

export function evaluatedRange(
  plan: Record<string, unknown>,
  nowMilliseconds = Date.now(),
): EvaluatedRange | null {
  const originalRange = rangeObject(plan);
  const resolvedRange = rangeObject(resolveQueryPlan(plan, nowMilliseconds));
  const start = finiteNumber(resolvedRange?.start_unix_nano);
  const end = finiteNumber(resolvedRange?.end_unix_nano);
  if (start === null || end === null || start >= end) return null;
  return {
    startUnixNano: start,
    endUnixNano: end,
    relative: parseRelativeRange(originalRange?.relative),
  };
}

export function describeQueryRange(
  plan: Record<string, unknown>,
  nowMilliseconds = Date.now(),
): string {
  const range = evaluatedRange(plan, nowMilliseconds);
  if (!range) return "Time range unavailable";
  const formatter = new Intl.DateTimeFormat(undefined, {
    month: "short",
    day: "numeric",
    year: "numeric",
    hour: "numeric",
    minute: "2-digit",
    timeZone: "UTC",
    timeZoneName: "short",
  });
  const start = formatter.format(new Date(range.startUnixNano / NANOSECONDS_PER_MILLISECOND));
  const end = formatter.format(new Date(range.endUnixNano / NANOSECONDS_PER_MILLISECOND));
  const prefix = range.relative
    ? `Rolling ${range.relative.amount} ${pluralize(range.relative.unit, range.relative.amount)}`
    : "Fixed range";
  return `${prefix} · ${start}–${end}`;
}

export function rowsAsObjects(result: QueryResult | null): Array<Record<string, unknown>> {
  if (!result || !Array.isArray(result.columns) || !Array.isArray(result.rows)) return [];
  return result.rows.map((row) =>
    Object.fromEntries(result.columns.map((column, index) => [column, row[index]])),
  );
}

export type RetentionCohort = {
  cohort: string;
  size: number;
  values: Map<number, number>;
};

export function retentionModel(result: QueryResult | null): {
  periods: number[];
  cohorts: RetentionCohort[];
} {
  const rows = rowsAsObjects(result);
  const validRows = rows.flatMap((row) => {
    const period = finiteNumber(row.period);
    const rate = finiteNumber(row.retention_rate);
    const size = finiteNumber(row.cohort_subjects);
    const cohort = typeof row.cohort_bucket === "string" ? row.cohort_bucket : "";
    if (!cohort || period === null || rate === null || size === null) return [];
    return [{ cohort, period, rate: Math.min(1, Math.max(0, rate)), size: Math.max(0, size) }];
  });
  const periods = [...new Set(validRows.map((row) => row.period))].sort((a, b) => a - b);
  const cohorts = [...new Set(validRows.map((row) => row.cohort))].map((cohort) => {
    const matching = validRows.filter((row) => row.cohort === cohort);
    return {
      cohort,
      size: matching[0]?.size ?? 0,
      values: new Map(matching.map((row) => [row.period, row.rate])),
    };
  });
  return { periods, cohorts };
}

export function seriesValues(result: QueryResult | null): Array<{
  label: string;
  value: number;
}> {
  if (!result) return [];
  return result.rows.flatMap((row, index) => {
    const numericIndex = [...row].map((value, cell) => ({ value, cell })).reverse()
      .find(({ value }) => typeof value === "number" && Number.isFinite(value));
    if (!numericIndex) return [];
    const labelValue = row.find((value, cell) => cell !== numericIndex.cell && typeof value === "string");
    return [{
      label: typeof labelValue === "string" && labelValue ? labelValue : `Point ${index + 1}`,
      value: numericIndex.value as number,
    }];
  });
}

function rangeObject(plan: Record<string, unknown>): Record<string, unknown> | null {
  const range = plan.range;
  return range && typeof range === "object" && !Array.isArray(range)
    ? range as Record<string, unknown>
    : null;
}

function parseRelativeRange(value: unknown): RelativeRange | null {
  if (!value || typeof value !== "object" || Array.isArray(value)) return null;
  const candidate = value as Record<string, unknown>;
  const amount = finiteNumber(candidate.amount);
  const unit = candidate.unit;
  if (
    amount === null
    || !Number.isInteger(amount)
    || amount < 1
    || amount > 744
    || (unit !== "hour" && unit !== "day" && unit !== "week")
  ) return null;
  return { amount, unit };
}

function finiteNumber(value: unknown): number | null {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

function pluralize(unit: RelativeRange["unit"], amount: number): string {
  return amount === 1 ? unit : `${unit}s`;
}
