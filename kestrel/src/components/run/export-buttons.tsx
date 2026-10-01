"use client";

import { Download } from "lucide-react";
import { useState } from "react";
import { toast } from "sonner";

import { errorMessage, getReportCsv } from "@/lib/client";
import { useRunStore } from "@/stores/run-store";
import type { RunReport } from "@/types/engine/RunReport";
import { Button } from "../ui/button";

/** `kestrel-load-1f2e3d4c.json`, matching the engine's CSV file names. */
const fileName = (report: RunReport, extension: string) =>
  `kestrel-${report.config.kind}-${report.runId.replaceAll("-", "").slice(0, 8)}.${extension}`;

const download = (contents: string, type: string, name: string) => {
  const url = URL.createObjectURL(new Blob([contents], { type }));
  const link = Object.assign(document.createElement("a"), { href: url, download: name });
  link.click();
  URL.revokeObjectURL(url);
};

/** Saves the shown report: the whole report as JSON (already redacted by the engine), or its
 * chartable table as CSV. */
export const ExportButtons = () => {
  const report = useRunStore((s) => s.report);
  const [busy, setBusy] = useState(false);
  if (!report) return null;

  const exportCsv = async () => {
    setBusy(true);
    try {
      download(await getReportCsv(report.runId), "text/csv", fileName(report, "csv"));
    } catch (err) {
      toast.error(errorMessage(err));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="flex items-center gap-2">
      <Button
        variant="outline"
        size="sm"
        onClick={() =>
          download(JSON.stringify(report, null, 2), "application/json", fileName(report, "json"))
        }
        title="The full report, as the engine stores it (secrets redacted)"
      >
        <Download className="size-3.5" /> JSON
      </Button>
      <Button
        variant="outline"
        size="sm"
        onClick={exportCsv}
        disabled={busy}
        title={
          report.complexity
            ? "One row per input size n"
            : "One row per 250 ms window: requests, errors, rps, p50/p99"
        }
      >
        <Download className="size-3.5" /> CSV
      </Button>
    </div>
  );
};
