"use client";

import { Plus, X } from "lucide-react";

import type { KeyValue } from "@/types/engine/KeyValue";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";

interface Props {
  rows: KeyValue[];
  onChange: (rows: KeyValue[]) => void;
  keyPlaceholder?: string;
}

/** Editable key/value rows with an enable toggle. Values may contain `{{templates}}`. */
export const KeyValueEditor = ({ rows, onChange, keyPlaceholder = "name" }: Props) => {
  const update = (i: number, patch: Partial<KeyValue>) =>
    onChange(rows.map((r, j) => (j === i ? { ...r, ...patch } : r)));

  return (
    <div className="flex flex-col gap-1.5">
      {rows.map((row, i) => (
        <div key={i} className="flex items-center gap-1.5">
          <Checkbox
            checked={row.enabled}
            onCheckedChange={(checked) => update(i, { enabled: !!checked })}
            aria-label="Enabled"
          />
          <Input
            className="flex-1"
            placeholder={keyPlaceholder}
            value={row.key}
            onChange={(e) => update(i, { key: e.target.value })}
          />
          <Input
            className="flex-2 font-mono"
            placeholder="value or {{var}}"
            value={row.value}
            onChange={(e) => update(i, { value: e.target.value })}
          />
          <button
            type="button"
            className="text-text-gray hover:text-primary"
            onClick={() => onChange(rows.filter((_, j) => j !== i))}
            aria-label="Remove"
          >
            <X className="size-4" />
          </button>
        </div>
      ))}
      <button
        type="button"
        className="text-text-gray hover:text-primary flex items-center gap-1 self-start text-xs"
        onClick={() => onChange([...rows, { key: "", value: "", enabled: true }])}
      >
        <Plus className="size-3.5" /> Add
      </button>
    </div>
  );
};
