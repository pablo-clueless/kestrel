"use client";

import { ArrowRight, Plus, X } from "lucide-react";

import type { ExtractSource } from "@/types/engine/ExtractSource";
import type { ExtractTarget } from "@/types/engine/ExtractTarget";
import type { Extract } from "@/types/engine/Extract";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { cn } from "@/lib/utils";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";

const XS = "text-xs md:text-xs";

const PATH_PLACEHOLDER: Record<ExtractSource, string> = {
  body: "data.token",
  header: "X-Request-Id",
  status: "—",
};

export const newRule = (): Extract => ({
  source: "body",
  path: "",
  target: "variable",
  name: "",
  enabled: true,
});

/** "After response" rules: values copied from each Send's response into the active environment. */
export const ExtractEditor = ({
  rules,
  onChange,
}: {
  rules: Extract[];
  onChange: (rules: Extract[]) => void;
}) => {
  const update = (i: number, patch: Partial<Extract>) =>
    onChange(rules.map((r, j) => (j === i ? { ...r, ...patch } : r)));

  return (
    <div className="flex flex-col gap-1.5">
      {rules?.map((rule, i) => (
        <div key={i} className="flex items-center gap-1.5">
          <Checkbox
            checked={rule.enabled}
            onCheckedChange={(checked) => update(i, { enabled: !!checked })}
            aria-label="Enabled"
          />
          <Select
            value={rule.source}
            onValueChange={(v) => update(i, { source: v as ExtractSource })}
          >
            <SelectTrigger className={cn("w-22 shrink-0", XS)} aria-label="Source">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="body">Body</SelectItem>
              <SelectItem value="header">Header</SelectItem>
              <SelectItem value="status">Status</SelectItem>
            </SelectContent>
          </Select>
          <Input
            className={cn("flex-2 font-mono", XS)}
            placeholder={PATH_PLACEHOLDER[rule.source]}
            disabled={rule.source === "status"}
            value={rule.source === "status" ? "" : rule.path}
            onChange={(e) => update(i, { path: e.target.value })}
            aria-label="Path"
          />
          <ArrowRight className="text-muted-foreground size-3.5 shrink-0" />
          <Select
            value={rule.target}
            onValueChange={(v) => update(i, { target: v as ExtractTarget })}
          >
            <SelectTrigger className={cn("w-24 shrink-0", XS)} aria-label="Save as">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="variable">Variable</SelectItem>
              <SelectItem value="secret">Secret</SelectItem>
            </SelectContent>
          </Select>
          <Input
            className={cn("flex-1 font-mono", XS)}
            placeholder="name"
            value={rule.name}
            onChange={(e) => update(i, { name: e.target.value })}
            aria-label="Name"
          />
          <button
            type="button"
            className="text-muted-foreground hover:text-primary"
            onClick={() => onChange(rules.filter((_, j) => j !== i))}
            aria-label="Remove rule"
          >
            <X className="size-4" />
          </button>
        </div>
      ))}
      <button
        type="button"
        className="text-muted-foreground hover:text-primary flex items-center gap-1 self-start text-xs"
        onClick={() => onChange([...(rules ?? []), newRule()])}
      >
        <Plus className="size-3.5" /> Add rule
      </button>
      <p className="text-muted-foreground text-xs">
        Runs after each Send (not during test runs) and saves into the active environment, usable as{" "}
        {"{{name}}"}. Body paths look like <code className="font-mono">data.token</code> or{" "}
        <code className="font-mono">items[0].id</code>; leave it empty for the whole body. Secrets
        are redacted from results.
      </p>
    </div>
  );
};
