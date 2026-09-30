"use client";

import { BookmarkPlus, Check, Copy, KeyRound, ShieldAlert, ShieldCheck, X } from "lucide-react";
import { useState } from "react";
import { toast } from "sonner";

import { SaveValueDialog, type SaveValueSeed } from "./save-value-dialog";
import { StatusBadge } from "@/components/shared/method-badge";
import { useWorkspaceStore } from "@/stores/workspace-store";
import { useSendEntry } from "@/stores/send-store";
import type { Saved } from "@/types/engine/Saved";
import { BarLoader } from "../shared";
import { cn } from "@/lib/utils";
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
  const endpointId = useWorkspaceStore((s) => s.selectedId);
  const { result, error, pending } = useSendEntry(endpointId);
  const [tab, setTab] = useState<Tab>("body");
  const [seed, setSeed] = useState<SaveValueSeed | null>(null);

  if (pending) return <BarLoader />;
  if (error) return <p className="text-destructive text-sm">{error}</p>;
  if (!result)
    return <p className="text-muted-foreground text-sm">Press Send to try the request once.</p>;

  const handleCopy = (value?: string) => {
    if (!value) return;
    window.navigator.clipboard
      .writeText(value)
      .then(() => toast.success("Copied"))
      .catch(() => toast.error("Copy failed"));
  };

  return (
    <div className="flex flex-col gap-3 text-sm">
      <div className="flex flex-wrap items-center gap-x-4 gap-y-1 tabular-nums">
        {result.status !== null ? (
          <StatusBadge status={result.status} />
        ) : (
          <span className="text-destructive font-medium">No response</span>
        )}
        <span>{result.totalMs.toFixed(1)} ms</span>
        <span className="text-muted-foreground">TTFB {result.ttfbMs.toFixed(1)} ms</span>
        <span className="text-muted-foreground">{formatBytes(result.bodyBytes)}</span>
      </div>
      {result.error && <p className="text-destructive">{result.error}</p>}
      {result.contract && (
        <p
          className={cn(
            "flex items-start gap-2 rounded-xs px-3 py-2 text-xs",
            result.contract.passed
              ? "bg-green-50 text-green-800 dark:bg-green-950 dark:text-green-300"
              : "bg-red-50 text-red-800 dark:bg-red-950 dark:text-red-300",
          )}
        >
          {result.contract.passed ? (
            <ShieldCheck className="size-3.5 shrink-0" />
          ) : (
            <ShieldAlert className="size-3.5 shrink-0" />
          )}
          <span className="break-all">Spec: {result.contract.message}</span>
        </p>
      )}
      {result.saved.length > 0 && <SavedList saved={result.saved} />}
      <div className="flex items-center justify-between gap-2">
        <Tabs<Tab>
          value={tab}
          onChange={setTab}
          tabs={[
            { id: "body", label: "Body" },
            { id: "headers", label: "Headers", count: result.responseHeaders.length },
            { id: "request", label: "Sent" },
          ]}
        />
        {result.status !== null && (
          <button
            className="text-muted-foreground hover:text-primary flex items-center gap-1 text-xs"
            onClick={() => setSeed({ source: tab === "headers" ? "header" : "body", path: "" })}
          >
            <BookmarkPlus className="size-3.5" /> Save value
          </button>
        )}
      </div>
      {tab === "body" && (
        <pre className="bg-muted relative max-h-80 overflow-auto rounded-xs p-3 font-mono text-xs whitespace-pre-wrap select-text">
          {result && result.body !== "" && (
            <button className="fixed top-2 right-2" onClick={() => handleCopy(result.body)}>
              <Copy className="size-4" />
            </button>
          )}
          {formatBody(result.body)}
          {result.bodyTruncated && <span className="text-muted-foreground">{"\n"}… truncated</span>}
        </pre>
      )}
      {tab === "headers" && (
        <HeaderTable
          headers={result.responseHeaders}
          onSave={(name) => setSeed({ source: "header", path: name })}
        />
      )}
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
      <SaveValueDialog seed={seed} sample={result} onClose={() => setSeed(null)} />
    </div>
  );
};

/** What the endpoint's "After response" rules saved on this Send. */
const SavedList = ({ saved }: { saved: Saved[] }) => (
  <ul className="flex flex-col gap-1 text-xs">
    {saved.map((s, i) => (
      <li key={i} className="flex min-w-0 items-center gap-1.5">
        {s.error === null ? (
          <Check className="size-3.5 shrink-0 text-green-600" />
        ) : (
          <X className="text-destructive size-3.5 shrink-0" />
        )}
        {s.target === "secret" && <KeyRound className="text-muted-foreground size-3 shrink-0" />}
        <span className="font-mono">{`{{${s.name}}}`}</span>
        <span
          className={cn(
            "truncate",
            s.error === null ? "text-muted-foreground" : "text-destructive",
          )}
        >
          {s.error ?? (s.target === "secret" ? "saved as secret" : `= ${s.value}`)}
        </span>
      </li>
    ))}
  </ul>
);

const HeaderTable = ({
  headers,
  onSave,
}: {
  headers: [string, string][];
  onSave?: (name: string) => void;
}) => (
  <table className="font-mono text-xs">
    <tbody>
      {headers.map(([k, v], i) => (
        <tr key={i} className="group align-top">
          <td className="text-muted-foreground pr-3 whitespace-nowrap">{k}</td>
          <td className="break-all">{v}</td>
          {onSave && (
            <td className="pl-2">
              <button
                className="text-muted-foreground hover:text-primary opacity-0 transition-opacity group-hover:opacity-100 focus-visible:opacity-100"
                onClick={() => onSave(k)}
                aria-label={`Save ${k}`}
                title="Save as variable or secret"
              >
                <BookmarkPlus className="size-3.5" />
              </button>
            </td>
          )}
        </tr>
      ))}
    </tbody>
  </table>
);
