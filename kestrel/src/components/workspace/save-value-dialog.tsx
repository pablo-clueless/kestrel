"use client";

import { useId, useMemo } from "react";
import { toast } from "sonner";

import { leafPaths, previewPick, REDACTED, suggestName } from "@/lib/extract";
import { useSelectedEndpoint, useWorkspaceStore } from "@/stores/workspace-store";
import type { ExtractSource } from "@/types/engine/ExtractSource";
import type { ExtractTarget } from "@/types/engine/ExtractTarget";
import type { Sample } from "@/types/engine/Sample";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { useValues } from "@/hooks/use-values";
import { cn } from "@/lib/utils";
import { Label } from "./fields";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";

export interface SaveValueSeed {
  source: ExtractSource;
  path: string;
}

/** Save one value from the last response now, or add it as an "After response" rule. */
export const SaveValueDialog = ({
  seed,
  sample,
  onClose,
}: {
  seed: SaveValueSeed | null;
  sample: Sample;
  onClose: () => void;
}) => (
  <Dialog open={seed !== null} onOpenChange={(open) => !open && onClose()}>
    <DialogContent>
      <DialogHeader>
        <DialogTitle>Save value from response</DialogTitle>
        <DialogDescription>
          Stored in the active environment and usable as {"{{name}}"}. Add it as a rule to refresh
          it on every Send, e.g. a login token.
        </DialogDescription>
      </DialogHeader>
      {/* Keyed so each open starts from its seed. */}
      {seed && (
        <SaveValueForm
          key={`${seed.source}:${seed.path}`}
          seed={seed}
          sample={sample}
          onClose={onClose}
        />
      )}
    </DialogContent>
  </Dialog>
);

const SaveValueForm = ({
  seed,
  sample,
  onClose,
}: {
  seed: SaveValueSeed;
  sample: Sample;
  onClose: () => void;
}) => {
  const env = useWorkspaceStore((s) => s.workspace?.activeEnvironment ?? null);
  const { setVar, setSecret, updateEndpoint } = useWorkspaceStore();
  const endpoint = useSelectedEndpoint();
  const listId = useId();
  const { values, set, patch } = useValues({
    initialValue: {
      source: seed.source,
      path: seed.path,
      target: "variable" as ExtractTarget,
      name: suggestName(seed.source, seed.path),
      // Stop following the path once the user types a name.
      nameEdited: false,
    },
  });
  const { source, path, target, name, nameEdited } = values;

  const paths = useMemo(() => leafPaths(sample.body), [sample.body]);
  const picked = previewPick(sample, source, path);
  const value = "value" in picked ? picked.value : null;
  const redacted = value?.includes(REDACTED) ?? false;
  const trimmedName = name.trim();

  const setPath = (p: string) =>
    patch({ path: p, ...(nameEdited ? {} : { name: suggestName(source, p) }) });

  const canSaveNow = env !== null && value !== null && !redacted && trimmedName !== "";
  const canAddRule = endpoint !== null && trimmedName !== "";

  const saveNow = async () => {
    if (!canSaveNow) return;
    if (target === "secret") await setSecret(env, trimmedName, value);
    else setVar(env, trimmedName, value);
  };

  const onSaveNow = async () => {
    await saveNow();
    toast.success(`Saved {{${trimmedName}}} to ${env}`);
    onClose();
  };

  const onAddRule = async () => {
    if (!endpoint) return;
    const rule = {
      source,
      path: source === "status" ? "" : path.trim(),
      target,
      name: trimmedName,
      enabled: true,
    };
    updateEndpoint(endpoint.id, { extract: [...endpoint.extract, rule] });
    await saveNow();
    toast.success(
      canSaveNow
        ? `Saved {{${trimmedName}}} and added a rule`
        : `Added a rule; it runs on the next Send`,
    );
    onClose();
  };

  return (
    <>
      <div className="flex flex-col gap-3 text-sm">
        <div className="grid grid-cols-[5rem_1fr] items-center gap-2">
          <Label>From</Label>
          <div className="flex gap-1.5">
            <Select
              value={source}
              onValueChange={(v) => {
                const s = v as ExtractSource;
                patch({ source: s, ...(nameEdited ? {} : { name: suggestName(s, path) }) });
              }}
            >
              <SelectTrigger className="w-24 shrink-0 text-xs md:text-xs">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="body">Body</SelectItem>
                <SelectItem value="header">Header</SelectItem>
                <SelectItem value="status">Status</SelectItem>
              </SelectContent>
            </Select>
            {source !== "status" && (
              <>
                <Input
                  className="flex-1 font-mono text-xs md:text-xs"
                  placeholder={
                    source === "body" ? "data.token (empty = whole body)" : "Header name"
                  }
                  value={path}
                  list={source === "body" ? listId : `${listId}-h`}
                  onChange={(e) => setPath(e.target.value)}
                  autoFocus
                />
                <datalist id={listId}>
                  {paths.map((p) => (
                    <option key={p} value={p} />
                  ))}
                </datalist>
                <datalist id={`${listId}-h`}>
                  {sample.responseHeaders.map(([k]) => (
                    <option key={k} value={k} />
                  ))}
                </datalist>
              </>
            )}
          </div>

          <Label>Value</Label>
          <p
            className={cn(
              "bg-muted max-h-24 overflow-auto rounded-xs px-2 py-1.5 font-mono text-xs break-all",
              value === null && "text-destructive",
            )}
          >
            {"error" in picked
              ? picked.error
              : value || <span className="text-muted-foreground">(empty)</span>}
          </p>

          <Label>Save as</Label>
          <div className="flex gap-1.5">
            <Select value={target} onValueChange={(v) => set("target", v as ExtractTarget)}>
              <SelectTrigger className="w-24 shrink-0 text-xs md:text-xs">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="variable">Variable</SelectItem>
                <SelectItem value="secret">Secret</SelectItem>
              </SelectContent>
            </Select>
            <Input
              className="flex-1 font-mono text-xs md:text-xs"
              placeholder="name"
              value={name}
              onChange={(e) => patch({ name: e.target.value, nameEdited: true })}
            />
          </div>
        </div>
        {env === null && (
          <p className="text-muted-foreground text-xs">
            Pick an environment in the header to save values. You can still add a rule.
          </p>
        )}
        {redacted && (
          <p className="text-muted-foreground text-xs">
            This value contains a redacted secret, so it can&apos;t be copied from here. Add it as a
            rule: the engine reads the real response on the next Send.
          </p>
        )}
      </div>
      <DialogFooter>
        <Button variant="outline" onClick={onAddRule} disabled={!canAddRule}>
          Save on every Send
        </Button>
        <Button onClick={onSaveNow} disabled={!canSaveNow}>
          Save now
        </Button>
      </DialogFooter>
    </>
  );
};
