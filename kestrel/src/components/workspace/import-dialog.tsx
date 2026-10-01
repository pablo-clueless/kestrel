"use client";

import { AlertTriangle, FileUp } from "lucide-react";
import { useMemo } from "react";

import type { ImportResult } from "@/types/engine/ImportResult";
import { MethodBadge } from "@/components/shared/method-badge";
import { useWorkspaceStore } from "@/stores/workspace-store";
import { errorMessage, importSpec } from "@/lib/client";
import { Textarea } from "@/components/ui/textarea";
import { Button } from "@/components/ui/button";
import { useValues } from "@/hooks/use-values";
import { Input } from "@/components/ui/input";
import { cn } from "@/lib/utils";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";

import { Label, Tabs } from "./fields";

type SourceTab = "file" | "paste" | "url";

/** Everything the dialog holds. Closing resets to this. */
const INITIAL = {
  tab: "file" as SourceTab,
  text: "",
  fileName: null as string | null,
  url: "",
  name: "",
  pending: false,
  error: null as string | null,
  result: null as ImportResult | null,
  dragging: false,
};

/** OpenAPI 3.0/3.1 or Swagger 2.0 (JSON or YAML) → a new collection. Two steps: source, then review. */
export const ImportDialog = ({ open, onClose }: { open: boolean; onClose: () => void }) => {
  const addImportedCollection = useWorkspaceStore((s) => s.addImportedCollection);
  const { values, set, patch, reset } = useValues({ initialValue: INITIAL });
  const { tab, text, fileName, url, name, pending, error, result, dragging } = values;

  const close = () => {
    reset();
    onClose();
  };

  const readFile = async (file: File | undefined) => {
    if (!file) return;
    patch({ fileName: file.name, text: await file.text(), error: null });
  };

  const canImport = tab === "url" ? url.trim().length > 0 : text.trim().length > 0;

  const run = async () => {
    patch({ pending: true, error: null });
    try {
      const source =
        tab === "url" ? { type: "url" as const, url } : { type: "text" as const, content: text };
      set("result", await importSpec({ source, name: name.trim() || null }));
    } catch (err) {
      set("error", errorMessage(err));
    } finally {
      set("pending", false);
    }
  };

  const add = () => {
    if (result) addImportedCollection(result.collection);
    close();
  };

  return (
    <Dialog open={open} onOpenChange={(o) => !o && close()}>
      <DialogContent className="sm:max-w-200">
        <DialogHeader>
          <DialogTitle>{result ? "Review import" : "Import API spec"}</DialogTitle>
          <DialogDescription>
            {result
              ? "Check what was found, then add it as a new collection."
              : "OpenAPI 3.0 / 3.1 or Swagger 2.0, as JSON or YAML. Each spec becomes a collection."}
          </DialogDescription>
        </DialogHeader>

        {result ? (
          <Review result={result} />
        ) : (
          <div className="flex flex-col gap-4 text-sm">
            <Tabs<SourceTab>
              value={tab}
              onChange={(t) => set("tab", t)}
              tabs={[
                { id: "file", label: "File" },
                { id: "paste", label: "Paste" },
                { id: "url", label: "URL" },
              ]}
            />
            {tab === "file" && (
              <label
                onDragOver={(e) => {
                  e.preventDefault();
                  set("dragging", true);
                }}
                onDragLeave={() => set("dragging", false)}
                onDrop={(e) => {
                  e.preventDefault();
                  set("dragging", false);
                  void readFile(e.dataTransfer.files[0]);
                }}
                className={cn(
                  "text-muted-foreground flex cursor-pointer flex-col items-center gap-2 rounded-xs border border-dashed px-4 py-8 text-center",
                  dragging && "border-primary bg-accent",
                )}
              >
                <FileUp className="size-6" />
                {fileName ? (
                  <span className="text-foreground font-medium">{fileName}</span>
                ) : (
                  <span>Drop a .json, .yaml or .yml file, or click to choose</span>
                )}
                <input
                  type="file"
                  accept=".json,.yaml,.yml,application/json,application/yaml,text/yaml"
                  className="hidden"
                  onChange={(e) => void readFile(e.target.files?.[0])}
                />
              </label>
            )}
            {tab === "paste" && (
              <Textarea
                className="h-60 max-w-3xl resize-none font-mono text-xs wrap-break-word"
                placeholder={"openapi: 3.0.3\ninfo:\n  title: My API\n…"}
                value={text}
                onChange={(e) => set("text", e.target.value)}
              />
            )}
            {tab === "url" && (
              <div className="flex flex-col gap-1.5">
                <Input
                  placeholder="https://api.example.com/openapi.json"
                  value={url}
                  onChange={(e) => set("url", e.target.value)}
                />
                <p className="text-muted-foreground text-xs">
                  The engine fetches it (up to 10 MB).
                </p>
              </div>
            )}
            <label className="flex flex-col gap-1.5">
              <Label>Collection name (optional)</Label>
              <Input
                placeholder="Defaults to the spec's title"
                value={name}
                onChange={(e) => set("name", e.target.value)}
              />
            </label>
            {error && <p className="text-destructive">{error}</p>}
          </div>
        )}

        <DialogFooter>
          {result ? (
            <>
              <Button variant="outline" onClick={() => set("result", null)}>
                Back
              </Button>
              <Button onClick={add} disabled={result.collection.endpoints.length === 0}>
                Add {result.collection.endpoints.length} endpoints
              </Button>
            </>
          ) : (
            <>
              <Button variant="outline" onClick={close}>
                Cancel
              </Button>
              <Button onClick={run} disabled={!canImport || pending}>
                {pending ? "Importing…" : "Import"}
              </Button>
            </>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
};

const Review = ({ result }: { result: ImportResult }) => {
  const { collection, format, warnings } = result;
  const groups = useMemo(() => {
    const counts = new Map<string, number>();
    for (const e of collection.endpoints)
      counts.set(e.group ?? "Ungrouped", (counts.get(e.group ?? "Ungrouped") ?? 0) + 1);
    return [...counts.entries()];
  }, [collection.endpoints]);
  const methods = useMemo(
    () => [...new Set(collection.endpoints.map((e) => e.method))],
    [collection.endpoints],
  );

  return (
    <div className="flex flex-col gap-4 text-sm">
      <dl className="grid grid-cols-2 gap-x-6 gap-y-3">
        <div>
          <dt className="text-muted-foreground text-xs">Collection</dt>
          <dd className="font-medium">{collection.name}</dd>
        </div>
        <div>
          <dt className="text-muted-foreground text-xs">Format</dt>
          <dd>{format}</dd>
        </div>
        <div className="col-span-2">
          <dt className="text-muted-foreground text-xs">Base URL (collection variable `base`)</dt>
          <dd className="truncate font-mono text-xs">{collection.vars.base || "— not set —"}</dd>
        </div>
      </dl>
      <div className="flex flex-col gap-2">
        <div className="flex items-center justify-between">
          <span className="text-muted-foreground text-xs">
            {collection.endpoints.length} endpoints
          </span>
          <span className="flex gap-1">
            {methods.map((m) => (
              <MethodBadge key={m} method={m} short />
            ))}
          </span>
        </div>
        <ul className="bg-muted flex max-h-36 flex-col gap-1 overflow-y-auto rounded-xs p-3 text-xs">
          {groups.map(([group, n]) => (
            <li key={group} className="flex justify-between">
              <span>{group}</span>
              <span className="text-muted-foreground">{n}</span>
            </li>
          ))}
        </ul>
      </div>
      {warnings.length > 0 && (
        <ul className="flex flex-col gap-1.5 rounded-xs bg-amber-50 p-3 text-xs text-amber-800 dark:bg-amber-950 dark:text-amber-300">
          {warnings.map((w) => (
            <li key={w} className="flex gap-2">
              <AlertTriangle className="mt-0.5 size-3.5 shrink-0" />
              {w}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
};
