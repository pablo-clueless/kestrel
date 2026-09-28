"use client";

import { FileUp, Loader2, Plus, X } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { toast } from "sonner";

import { errorMessage, uploadFile } from "@/lib/client";
import type { FieldKind } from "@/types/engine/FieldKind";
import type { FormField } from "@/types/engine/FormField";
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

const formatBytes = (n: number) =>
  n < 1024
    ? `${n} B`
    : n < 1024 * 1024
      ? `${(n / 1024).toFixed(1)} KB`
      : `${(n / 1024 / 1024).toFixed(1)} MB`;

const newField = (): FormField => ({ key: "", kind: "text", value: "", file: null, enabled: true });

/** Multipart fields: each row is text (a template) or a file uploaded to the engine. */
export const FormFieldEditor = ({
  rows,
  onChange,
}: {
  rows: FormField[];
  onChange: (rows: FormField[]) => void;
}) => {
  const update = (i: number, patch: Partial<FormField>) =>
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
            className={cn("flex-1", XS)}
            placeholder="field"
            value={row.key}
            onChange={(e) => update(i, { key: e.target.value })}
          />
          <Select value={row.kind} onValueChange={(v) => update(i, { kind: v as FieldKind })}>
            <SelectTrigger className={cn("w-20 shrink-0", XS)} aria-label="Field type">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="text">Text</SelectItem>
              <SelectItem value="file">File</SelectItem>
            </SelectContent>
          </Select>
          {row.kind === "text" ? (
            <Input
              className={cn("flex-2 font-mono", XS)}
              placeholder="value or {{var}}"
              value={row.value}
              onChange={(e) => update(i, { value: e.target.value })}
            />
          ) : (
            <FilePicker row={row} onPicked={(file) => update(i, { file })} />
          )}
          <button
            type="button"
            className="text-muted-foreground hover:text-primary"
            onClick={() => onChange(rows.filter((_, j) => j !== i))}
            aria-label="Remove"
          >
            <X className="size-4" />
          </button>
        </div>
      ))}
      <button
        type="button"
        className="text-muted-foreground hover:text-primary flex items-center gap-1 self-start text-xs"
        onClick={() => onChange([...rows, newField()])}
      >
        <Plus className="size-3.5" /> Add
      </button>
    </div>
  );
};

/** Uploads the chosen file to the engine; the field stores the returned reference. */
const FilePicker = ({
  row,
  onPicked,
}: {
  row: FormField;
  onPicked: (file: FormField["file"]) => void;
}) => {
  const input = useRef<HTMLInputElement>(null);
  const [uploading, setUploading] = useState(false);
  // Uploads are slow; apply the result to the rows as they are then, not as they were at click.
  const latestOnPicked = useRef(onPicked);
  useEffect(() => {
    latestOnPicked.current = onPicked;
  });

  const pick = async (file: File | undefined) => {
    if (!file) return;
    setUploading(true);
    try {
      const uploaded = await uploadFile(file);
      latestOnPicked.current(uploaded);
    } catch (err) {
      toast.error(`Upload failed: ${errorMessage(err)}`);
    } finally {
      setUploading(false);
      // Allow picking the same file again.
      if (input.current) input.current.value = "";
    }
  };

  return (
    <div className="flex min-w-0 flex-2 items-center gap-1.5">
      <input
        ref={input}
        type="file"
        className="hidden"
        onChange={(e) => void pick(e.target.files?.[0])}
      />
      <button
        type="button"
        disabled={uploading}
        onClick={() => input.current?.click()}
        className={cn(
          "border-input hover:border-primary flex h-8 min-w-0 flex-1 items-center gap-1.5 rounded-xs border border-dashed px-2 text-left",
          XS,
          !row.file && "text-muted-foreground",
        )}
        title={row.file ? `${row.file.name} · ${row.file.contentType}` : undefined}
      >
        {uploading ? (
          <Loader2 className="size-3.5 shrink-0 animate-spin" />
        ) : (
          <FileUp className="size-3.5 shrink-0" />
        )}
        {uploading ? (
          "Uploading…"
        ) : row.file ? (
          <>
            <span className="truncate">{row.file.name}</span>
            <span className="text-muted-foreground shrink-0">{formatBytes(row.file.size)}</span>
          </>
        ) : (
          "Choose file…"
        )}
      </button>
      {row.file && !uploading && (
        <button
          type="button"
          className="text-muted-foreground hover:text-primary shrink-0 text-xs"
          onClick={() => onPicked(null)}
        >
          Clear
        </button>
      )}
    </div>
  );
};
