"use client";

import { useMemo } from "react";
import {
  Bar,
  BarChart,
  CartesianGrid,
  ComposedChart,
  Legend,
  Line,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from "recharts";

import { useRunStore } from "@/stores/run-store";

// Theme tokens, so charts follow light/dark mode with the rest of the page.
const MUTED = "var(--muted-foreground)";
const tick = { fill: MUTED, fontSize: 11 };
const tooltipStyle = {
  contentStyle: {
    background: "var(--popover)",
    border: "1px solid var(--border)",
    borderRadius: 8,
    color: "var(--popover-foreground)",
    fontSize: 12,
  },
  labelStyle: { color: MUTED },
};
const legendProps = {
  verticalAlign: "top" as const,
  align: "left" as const,
  iconType: "circle" as const,
  iconSize: 8,
  wrapperStyle: { fontSize: 12, paddingBottom: 12, color: MUTED },
};

const SERIES = {
  p50: "#3b82f6",
  p99: "var(--primary)",
  rps: "#16a34a",
  dropped: "#dc2626",
};

/** Latency (left axis, ms) and throughput (right axis, req/s) over the run, one bucket per 250 ms.
 * The engine sends ≤ 4 buckets/s, so animation stays off. */
export const LiveChart = () => {
  const buckets = useRunStore((s) => s.buckets);
  const isLoad = useRunStore((s) => s.config?.kind === "load");
  const data = useMemo(
    () =>
      buckets.map((b) => ({
        t: b.tMs / 1000,
        p50: Number(b.p50Ms.toFixed(2)),
        p99: Number(b.p99Ms.toFixed(2)),
        rps: Math.round(b.rps),
        // Per second, like rps: a bucket is 250 ms.
        dropped: b.dropped * 4,
      })),
    [buckets],
  );

  if (!data.length) {
    return (
      <div className="text-muted-foreground flex h-72 items-center justify-center rounded-lg border border-dashed text-sm">
        Start a run to see latency and throughput live.
      </div>
    );
  }

  return (
    <div className="h-72 min-h-0">
      <ResponsiveContainer width="100%" height="100%">
        <ComposedChart data={data} margin={{ top: 0, right: 0, bottom: 0, left: -8 }}>
          <CartesianGrid vertical={false} stroke="var(--border)" />
          <XAxis
            dataKey="t"
            type="number"
            domain={["dataMin", "dataMax"]}
            tickFormatter={(t: number) => `${t.toFixed(0)}s`}
            tick={tick}
            axisLine={false}
            tickLine={false}
          />
          <YAxis yAxisId="ms" unit=" ms" tick={tick} axisLine={false} tickLine={false} width={64} />
          <YAxis yAxisId="rps" orientation="right" tick={tick} axisLine={false} tickLine={false} width={48} />
          <Tooltip labelFormatter={(t) => `${t}s`} {...tooltipStyle} />
          <Legend {...legendProps} />
          <Line yAxisId="ms" dataKey="p50" name="p50 (ms)" stroke={SERIES.p50} dot={false} strokeWidth={2} isAnimationActive={false} />
          <Line yAxisId="ms" dataKey="p99" name="p99 (ms)" stroke={SERIES.p99} dot={false} strokeWidth={2} isAnimationActive={false} />
          <Line yAxisId="rps" dataKey="rps" name="req/s" stroke={SERIES.rps} dot={false} strokeWidth={2} isAnimationActive={false} />
          {isLoad && (
            <Line
              yAxisId="rps"
              dataKey="dropped"
              name="dropped/s"
              stroke={SERIES.dropped}
              dot={false}
              strokeWidth={2}
              isAnimationActive={false}
            />
          )}
        </ComposedChart>
      </ResponsiveContainer>
    </div>
  );
};

/** Distribution of all requests in the finished run's report. */
export const DistributionChart = () => {
  const histogram = useRunStore((s) => s.report?.histogram);
  const data = useMemo(
    () => (histogram ?? []).map((b) => ({ ms: Number(b.loMs.toFixed(1)), count: b.count })),
    [histogram],
  );
  if (!data.length) return null;
  return (
    <div className="h-52 min-h-0">
      <ResponsiveContainer width="100%" height="100%">
        <BarChart data={data} margin={{ top: 4, right: 0, bottom: 0, left: -20 }} barCategoryGap={1}>
          <CartesianGrid vertical={false} stroke="var(--border)" />
          <XAxis dataKey="ms" tick={tick} unit="ms" minTickGap={16} axisLine={false} tickLine={false} />
          <YAxis tick={tick} allowDecimals={false} axisLine={false} tickLine={false} />
          <Tooltip labelFormatter={(v) => `≥ ${v} ms`} cursor={{ fill: "var(--muted)" }} {...tooltipStyle} />
          <Bar dataKey="count" name="requests" fill="var(--primary)" radius={[3, 3, 0, 0]} isAnimationActive={false} />
        </BarChart>
      </ResponsiveContainer>
    </div>
  );
};
