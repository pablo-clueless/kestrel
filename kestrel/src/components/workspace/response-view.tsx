"use client";

import { useState } from "react";

import { cn } from "@/lib/utils";
import { useSendStore } from "@/stores/send-store";

import { Tabs } from "./fields";

type Tab = "body" | "headers" | "request";

const formatBody = (body: string) => {
  try {
    return JSON.stringify(JSON.parse(body), null, 2);
  } catch {
    return body;
  }
};

const formatBytes = (n: number) => (n < 1024 ? `${n} B` : `${(n / 1024).toFixed(1)} KB`);

/** Result of the last "Send". Secrets are already redacted by the engine. */
export const ResponseView = () => {
  const { result, error, pending } = useSendStore();
  const [tab, setTab] = useState<Tab>("body");

  if (pending) return <p className="text-text-gray text-sm">Sending…</p>;
  if (error) return <p className="text-sm text-red-600">{error}</p>;
  if (!result) return <p className="text-text-gray text-sm">Press Send to try the request once.</p>;

  const ok = result.status !== null && result.status < 400;
  return (
    <div className="flex flex-col gap-3 text-sm">
      <div className="flex flex-wrap items-center gap-x-4 gap-y-1 font-mono">
        <span className={cn("font-bold", ok ? "text-green-600" : "text-red-600")}>
          {result.status ?? "—"}
        </span>
        <span>{result.totalMs.toFixed(1)} ms</span>
        <span className="text-text-gray">TTFB {result.ttfbMs.toFixed(1)} ms</span>
        <span className="text-text-gray">{formatBytes(result.bodyBytes)}</span>
      </div>
      {result.error && <p className="text-red-600">{result.error}</p>}

      <Tabs<Tab>
        value={tab}
        onChange={setTab}
        tabs={[
          { id: "body", label: "Body" },
          { id: "headers", label: "Headers", count: result.responseHeaders.length },
          { id: "request", label: "Sent" },
        ]}
      />
      {tab === "body" && (
        <pre className="max-h-80 overflow-auto font-mono text-xs whitespace-pre-wrap">
          {formatBody(result.body)}
          {result.bodyTruncated && <span className="text-text-gray">{"\n"}… truncated</span>}
        </pre>
      )}
      {tab === "headers" && <HeaderTable headers={result.responseHeaders} />}
      {tab === "request" && (
        <div className="flex flex-col gap-2">
          <p className="font-mono text-xs break-all">
            {result.request.method} {result.request.url}
          </p>
          <HeaderTable headers={result.request.headers} />
          {result.request.body && (
            <pre className="font-mono text-xs whitespace-pre-wrap">{result.request.body}</pre>
          )}
        </div>
      )}
    </div>
  );
};

const HeaderTable = ({ headers }: { headers: [string, string][] }) => (
  <table className="font-mono text-xs">
    <tbody>
      {headers.map(([k, v], i) => (
        <tr key={i} className="align-top">
          <td className="text-text-gray pr-3 whitespace-nowrap">{k}</td>
          <td className="break-all">{v}</td>
        </tr>
      ))}
    </tbody>
  </table>
);
