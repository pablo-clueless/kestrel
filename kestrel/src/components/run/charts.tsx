"use client";

import { useMemo } from "react";
import {
  Area,
  AreaChart,
  Bar,
  BarChart,
  CartesianGrid,
  Legend,
  Line,
  LineChart,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from "recharts";

import { useRunStore } from "@/stores/run-store";

const useChartData = () => {
  const buckets = useRunStore((s) => s.buckets);
  return useMemo(
    () =>
      buckets.map((b) => ({
        t: b.tMs / 1000,
        p50: Number(b.p50Ms.toFixed(1)),
        p99: Number(b.p99Ms.toFixed(1)),
        rps: Math.round(b.rps),
        // Per-second, like rps: a bucket is 250 ms.
        dropped: b.dropped * 4,
      })),
    [buckets],
  );
};

// Theme tokens, so charts follow light/dark mode with the rest of the page.
const MUTED = "var(--text-gray)";
export const tick = { fill: MUTED };
export const tooltipStyle = {
  contentStyle: {
    background: "var(--snow)",
    border: "1px solid var(--secondary-4)",
    color: "var(--text-black)",
  },
  labelStyle: { color: MUTED },
};

const xAxis = (
  <XAxis
    dataKey="t"
    type="number"
    domain={["dataMin", "dataMax"]}
    tickFormatter={(t: number) => `${t.toFixed(0)}s`}
    tick={tick}
    fontSize={12}
  />
);

/** p50 / p99 over time. Charts are fed ≤ 4 buckets/s by the engine, so animation stays off. */
export const LatencyChart = () => {
  const data = useChartData();
  return (
    <div className="h-64 min-h-0">
      <ResponsiveContainer width="100%" height="100%">
        <LineChart data={data} margin={{ top: 8, right: 8, bottom: 0, left: -12 }}>
          <CartesianGrid strokeDasharray="3 3" strokeOpacity={0.3} />
          {xAxis}
          <YAxis unit="ms" fontSize={12} tick={tick} />
          <Tooltip labelFormatter={(t) => `${t}s`} {...tooltipStyle} />
          <Legend wrapperStyle={{ color: MUTED }} />
          <Line dataKey="p50" name="p50" stroke="var(--void-100)" dot={false} isAnimationActive={false} />
          <Line dataKey="p99" name="p99" stroke="var(--primary)" dot={false} isAnimationActive={false} />
        </LineChart>
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
  if (!data.length) return <p className="text-text-gray text-sm">Appears when a latency run finishes.</p>;
  return (
    <div className="h-48 min-h-0">
      <ResponsiveContainer width="100%" height="100%">
        <BarChart data={data} margin={{ top: 8, right: 8, bottom: 0, left: -20 }} barCategoryGap={1}>
          <XAxis dataKey="ms" fontSize={10} tick={tick} unit="ms" minTickGap={16} />
          <YAxis fontSize={10} tick={tick} allowDecimals={false} />
          <Tooltip labelFormatter={(v) => `≥ ${v} ms`} {...tooltipStyle} />
          <Bar dataKey="count" name="requests" fill="var(--primary)" isAnimationActive={false} />
        </BarChart>
      </ResponsiveContainer>
    </div>
  );
};

export const ThroughputChart = () => {
  const data = useChartData();
  return (
    <div className="h-64 min-h-0">
      <ResponsiveContainer width="100%" height="100%">
        <AreaChart data={data} margin={{ top: 8, right: 8, bottom: 0, left: -12 }}>
          <CartesianGrid strokeDasharray="3 3" strokeOpacity={0.3} />
          {xAxis}
          <YAxis fontSize={12} tick={tick} />
          <Tooltip labelFormatter={(t) => `${t}s`} {...tooltipStyle} />
          <Area
            dataKey="rps"
            name="req/s"
            stroke="var(--primary)"
            fill="var(--primary)"
            fillOpacity={0.15}
            isAnimationActive={false}
          />
          <Area
            dataKey="dropped"
            name="dropped/s"
            stroke="#dc2626"
            fill="#dc2626"
            fillOpacity={0.15}
            isAnimationActive={false}
          />
        </AreaChart>
      </ResponsiveContainer>
    </div>
  );
};
