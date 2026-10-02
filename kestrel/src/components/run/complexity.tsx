"use client";

import { useMemo } from "react";
import {
  Area,
  CartesianGrid,
  ComposedChart,
  Legend,
  Line,
  ResponsiveContainer,
  Scatter,
  Tooltip,
  XAxis,
  YAxis,
} from "recharts";

import type { ComplexityResult } from "@/types/engine/ComplexityResult";
import type { ComplexityPoint } from "@/types/engine/ComplexityPoint";
import { StatGrid, StatTile } from "@/components/shared/stat-tile";
import type { Confidence } from "@/types/engine/Confidence";
import type { ModelFit } from "@/types/engine/ModelFit";
import type { Model } from "@/types/engine/Model";
import { useRunStore } from "@/stores/run-store";
import { cn } from "@/lib/utils";

import { legendProps, tick, tooltipStyle } from "./charts";

const LABELS: Record<Model, string> = {
  constant: "O(1)",
  log: "O(log n)",
  linear: "O(n)",
  linearithmic: "O(n log n)",
  quadratic: "O(n²)",
  cubic: "O(n³)",
};

/** Mirrors `Model::f` in the engine. */
const f = (model: Model, n: number) => {
  switch (model) {
    case "constant":
      return 0;
    case "log":
      return Math.log(n);
    case "linear":
      return n;
    case "linearithmic":
      return n * Math.log(n);
    case "quadratic":
      return n * n;
    case "cubic":
      return n * n * n;
  }
};

const CONFIDENCE_STYLE: Record<Confidence, string> = {
  high: "bg-green-50 text-green-700 dark:bg-green-950 dark:text-green-400",
  medium: "bg-blue-50 text-blue-700 dark:bg-blue-950 dark:text-blue-400",
  low: "bg-amber-50 text-amber-700 dark:bg-amber-950 dark:text-amber-400",
  inconclusive: "bg-muted text-muted-foreground",
};

const fmtN = (n: number) => (n >= 1000 ? `${(n / 1000).toLocaleString()}k` : String(n));
const fmtMs = (v: number) => (v < 1 ? v.toFixed(3) : v < 100 ? v.toFixed(2) : v.toFixed(0));

/** Medians per size with their p25–p75 band, on log–log axes, plus optional fitted curves. */
const SizeChart = ({ points, fits }: { points: ComplexityPoint[]; fits?: ModelFit[] }) => {
  const data = useMemo(() => {
    if (!points.length) return [];
    // Fitted curves are drawn on a denser log grid than the measured sizes.
    const min = points[0].n;
    const max = points[points.length - 1].n;
    const grid = new Set(points.map((p) => p.n));
    for (let i = 0; i <= 40; i++) grid.add(Math.round(min * Math.pow(max / min, i / 40)));
    const byN = new Map(points.map((p) => [p.n, p]));
    return [...grid]
      .sort((a, b) => a - b)
      .map((n) => {
        const p = byN.get(n);
        const row: Record<string, number | [number, number] | undefined> = {
          n,
          median: p?.medianMs,
          band: p ? [Math.max(p.p25Ms, 1e-4), Math.max(p.p75Ms, 1e-4)] : undefined,
        };
        fits?.forEach((fit, i) => {
          row[`fit${i}`] = Math.max(fit.a + fit.b * f(fit.model, n), 1e-4);
        });
        return row;
      });
  }, [points, fits]);

  if (!points.length) {
    return (
      <div className="text-muted-foreground flex h-72 items-center justify-center border border-dashed text-sm">
        Waiting for the first round…
      </div>
    );
  }

  return (
    <div className="h-72 min-h-0">
      <ResponsiveContainer width="100%" height="100%">
        <ComposedChart data={data} margin={{ top: 0, right: 8, bottom: 0, left: -4 }}>
          <CartesianGrid stroke="var(--border)" />
          <XAxis
            dataKey="n"
            type="number"
            scale="log"
            domain={["dataMin", "dataMax"]}
            tickFormatter={fmtN}
            tick={tick}
            axisLine={false}
            tickLine={false}
            label={{
              value: "n",
              position: "insideBottomRight",
              offset: -2,
              fill: "var(--muted-foreground)",
              fontSize: 11,
            }}
          />
          <YAxis
            type="number"
            scale="log"
            domain={["auto", "auto"]}
            allowDataOverflow
            tickFormatter={(v: number) => `${fmtMs(v)}`}
            unit=" ms"
            tick={tick}
            axisLine={false}
            tickLine={false}
            width={72}
          />
          <Tooltip
            {...tooltipStyle}
            labelFormatter={(n) => `n = ${Number(n).toLocaleString()}`}
            formatter={(v, name) =>
              Array.isArray(v)
                ? [`${fmtMs(v[0])} – ${fmtMs(v[1])} ms`, name]
                : [`${fmtMs(Number(v))} ms`, name]
            }
          />
          <Legend {...legendProps} />
          <Area
            dataKey="band"
            name="p25–p75"
            stroke="none"
            fill="var(--primary)"
            fillOpacity={0.12}
            isAnimationActive={false}
            connectNulls
          />
          {fits?.map((fit, i) => (
            <Line
              key={fit.model}
              dataKey={`fit${i}`}
              name={`${LABELS[fit.model]} fit`}
              stroke={i === 0 ? "#3b82f6" : "var(--muted-foreground)"}
              strokeDasharray={i === 0 ? undefined : "4 4"}
              strokeWidth={i === 0 ? 2 : 1.5}
              dot={false}
              isAnimationActive={false}
            />
          ))}
          <Scatter dataKey="median" name="median" fill="var(--primary)" isAnimationActive={false} />
        </ComposedChart>
      </ResponsiveContainer>
    </div>
  );
};

/** Live view while a Big-O sweep runs. */
export const ComplexityLive = () => {
  const progress = useRunStore((s) => s.complexity);
  const running = useRunStore((s) => s.status === "running");
  const done = progress ? Math.min(progress.round, progress.rounds) : 0;
  const pct = progress && progress.rounds ? (done / progress.rounds) * 100 : 0;

  return (
    <div className="flex flex-col gap-5">
      <div className="flex flex-col gap-2">
        <div className="text-muted-foreground flex justify-between text-xs">
          <span>
            Round {done} of {progress?.rounds ?? "…"} · sizes sampled in shuffled order each round
          </span>
          <span>{progress?.points.length ?? 0} sizes with results</span>
        </div>
        <div className="bg-muted h-1.5 overflow-hidden rounded-full">
          <div
            className={cn("bg-primary h-full transition-all", running && "animate-pulse")}
            style={{ width: `${pct}%` }}
          />
        </div>
      </div>
      <SizeChart points={progress?.points ?? []} />
    </div>
  );
};

/** Final verdict, fits and per-size numbers. */
export const ComplexityResults = ({ result }: { result: ComplexityResult }) => {
  const { analysis, points } = result;
  // Best fit plus a close runner-up, drawn over the measurements.
  const shown = analysis.fits.filter(
    (fit, i) =>
      i === 0 || (analysis.alsoPlausible !== null && fit.model === analysis.alsoPlausible),
  );
  const bestAicc = analysis.fits[0]?.aicc ?? 0;
  const last = points[points.length - 1];

  return (
    <div className="flex flex-col gap-6">
      <div className="flex flex-wrap items-end justify-between gap-4">
        <div className="flex flex-col gap-1.5">
          <span className="text-muted-foreground text-xs">Empirical complexity</span>
          <span className="text-3xl leading-none font-semibold">{analysis.verdict}</span>
        </div>
        <span
          className={cn(
            "rounded-full px-2.5 py-1 text-xs font-medium capitalize",
            CONFIDENCE_STYLE[analysis.confidence],
          )}
        >
          {analysis.confidence === "inconclusive"
            ? "Inconclusive"
            : `${analysis.confidence} confidence`}
        </span>
      </div>
      <p className="text-muted-foreground text-sm">{analysis.explanation}</p>

      <StatGrid cols={4}>
        <StatTile
          label="Sizes"
          value={String(points.length)}
          info="Sizes with at least one successful sample"
        />
        <StatTile label="Largest n" value={last ? last.n.toLocaleString() : "–"} />
        <StatTile
          label="Log-log slope"
          value={analysis.slope == null ? "–" : analysis.slope.toFixed(2)}
          info="Upper half of the range: ≈0 constant, ≈1 linear, ≈2 quadratic"
        />
        <StatTile
          label="Latency at largest n"
          value={last ? fmtMs(last.medianMs) : "–"}
          unit="ms"
        />
      </StatGrid>

      <div className="border-t pt-5">
        <SizeChart points={points} fits={shown} />
      </div>

      <div className="grid grid-cols-2 gap-6 border-t pt-5">
        <table className="w-full self-start text-sm tabular-nums">
          <thead className="text-muted-foreground text-xs">
            <tr className="border-b">
              <th className="py-2 text-left font-normal">Model</th>
              <th className="py-2 text-right font-normal">R²</th>
              <th
                className="py-2 text-right font-normal"
                title="Difference from the best AICc; under ~4 is a close call"
              >
                ΔAICc
              </th>
            </tr>
          </thead>
          <tbody>
            {analysis.fits.map((fit, i) => (
              <tr
                key={fit.model}
                className={cn("border-b last:border-0", i === 0 && "font-medium")}
              >
                <td className="py-1.5">{LABELS[fit.model]}</td>
                <td className="py-1.5 text-right">{fit.r2.toFixed(3)}</td>
                <td className="py-1.5 text-right">
                  {i === 0 ? "best" : `+${(fit.aicc - bestAicc).toFixed(1)}`}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
        <div className="max-h-64 overflow-y-auto">
          <table className="w-full text-sm tabular-nums">
            <thead className="text-muted-foreground bg-card sticky top-0 text-xs">
              <tr className="border-b">
                <th className="py-2 text-left font-normal">n</th>
                <th className="py-2 text-right font-normal">Median</th>
                <th className="py-2 text-right font-normal">IQR</th>
                <th className="py-2 text-right font-normal">Body</th>
              </tr>
            </thead>
            <tbody>
              {points.map((p) => (
                <tr key={p.n} className="border-b last:border-0">
                  <td className="py-1.5">{p.n.toLocaleString()}</td>
                  <td className="py-1.5 text-right">{fmtMs(p.medianMs)} ms</td>
                  <td className="text-muted-foreground py-1.5 text-right">
                    {fmtMs(p.p75Ms - p.p25Ms)}
                  </td>
                  <td className="text-muted-foreground py-1.5 text-right">
                    {p.requestBytes ? `${(p.requestBytes / 1024).toFixed(1)} KB` : "–"}
                    {p.errors ? (
                      <span className="text-destructive ml-1">· {p.errors} err</span>
                    ) : null}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </div>
    </div>
  );
};
