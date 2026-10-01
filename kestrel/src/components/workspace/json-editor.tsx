"use client";

import { useDeferredValue, useId, useRef } from "react";
import { CircleCheck, CircleAlert } from "lucide-react";

import { checkJson, formatJson } from "@/lib/json-text";
import { Textarea } from "@/components/ui/textarea";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";

const PAIRS: Record<string, string> = { "{": "}", "[": "]", "(": ")", '"': '"' };
const CLOSERS = new Set(["}", "]", ")"]);
const INDENT = "  ";

/** Whether `offset` is inside a string on its line (JSON strings can't span lines). */
const inString = (text: string, offset: number) => {
  let inside = false;
  for (let i = text.lastIndexOf("\n", offset - 1) + 1; i < offset; i++) {
    if (text[i] === "\\" && inside) i++;
    else if (text[i] === '"') inside = !inside;
  }
  return inside;
};

/**
 * Replaces the textarea's selection with `text` the way typing would, so it lands in the browser's
 * undo history (Ctrl+Z works), then selects `[from, to]`. `execCommand` is deprecated but still the
 * only way to edit a textarea undoably; if it's refused, edit the value directly.
 */
const insert = (
  ta: HTMLTextAreaElement,
  text: string,
  from: number,
  to: number,
  onChange: (v: string) => void,
) => {
  ta.focus();
  const done = document.execCommand(text ? "insertText" : "delete", false, text);
  if (!done) {
    ta.setRangeText(text);
    onChange(ta.value);
  }
  ta.setSelectionRange(from, to);
};

interface Props {
  value: string;
  onChange: (value: string) => void;
  placeholder?: string;
  className?: string;
}

/**
 * The JSON body editor: brackets and quotes close themselves, Enter keeps the indentation, and the
 * body is checked as you type (templates like `{{seq}}` count as values). Format (Shift+Alt+F)
 * re-indents it without changing any value.
 */
export const JsonEditor = ({ value, onChange, placeholder, className }: Props) => {
  const statusId = useId();
  const ref = useRef<HTMLTextAreaElement>(null);
  const checked = useDeferredValue(value);
  const problem = checkJson(checked);
  const formatted = problem ? null : formatJson(checked);
  const canFormat = formatted !== null && formatted !== checked;

  const format = (ta: HTMLTextAreaElement) => {
    const next = formatJson(ta.value);
    if (next === null || next === ta.value) return;
    ta.setSelectionRange(0, ta.value.length);
    insert(ta, next, 0, 0, onChange);
  };

  const onKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    const ta = e.currentTarget;
    if (e.shiftKey && e.altKey && e.code === "KeyF") {
      e.preventDefault();
      format(ta);
      return;
    }
    if (e.nativeEvent.isComposing || e.ctrlKey || e.metaKey || e.altKey) return;
    const { value: text, selectionStart: start, selectionEnd: end } = ta;
    const before = text[start - 1] ?? "";
    const after = text[end] ?? "";
    const collapsed = start === end;

    // Typing the closer that's already next to the caret steps over it.
    if (
      collapsed &&
      after === e.key &&
      (CLOSERS.has(e.key) || (e.key === '"' && inString(text, start)))
    ) {
      e.preventDefault();
      ta.setSelectionRange(start + 1, start + 1);
      return;
    }

    if (e.key in PAIRS) {
      const close = PAIRS[e.key];
      if (!collapsed) {
        // Wrap the selection, keeping it selected.
        e.preventDefault();
        insert(ta, e.key + text.slice(start, end) + close, start + 1, end + 1, onChange);
        return;
      }
      // Only before whitespace, a closer or a separator, so typing in front of a value doesn't
      // leave a stray closer behind.
      const roomy =
        after === "" || /\s/.test(after) || CLOSERS.has(after) || after === "," || after === ":";
      if (!roomy) return;
      if (e.key === '"' && (inString(text, start) || before === "\\" || /\w/.test(before))) return;
      e.preventDefault();
      insert(ta, e.key + close, start + 1, start + 1, onChange);
      return;
    }

    // Backspace inside an empty pair removes both halves.
    if (e.key === "Backspace" && collapsed && before && PAIRS[before] === after) {
      e.preventDefault();
      ta.setSelectionRange(start - 1, start + 1);
      insert(ta, "", start - 1, start - 1, onChange);
      return;
    }

    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      const lineStart = text.lastIndexOf("\n", start - 1) + 1;
      const indent = /^[ \t]*/.exec(text.slice(lineStart))?.[0] ?? "";
      const opens = before === "{" || before === "[";
      if (opens && collapsed && PAIRS[before] === after) {
        // Between a fresh pair: the caret goes on its own indented line, the closer below it.
        const inner = "\n" + indent + INDENT;
        insert(ta, inner + "\n" + indent, start + inner.length, start + inner.length, onChange);
      } else {
        const next = "\n" + indent + (opens ? INDENT : "");
        insert(ta, next, start + next.length, start + next.length, onChange);
      }
    }
  };

  const shown = problem && checked === value ? problem : null;
  return (
    <div className="flex flex-col gap-1.5">
      <Textarea
        ref={ref}
        className={cn("min-h-40 resize-none font-mono", className)}
        value={value}
        placeholder={placeholder}
        spellCheck={false}
        autoCapitalize="off"
        autoCorrect="off"
        aria-invalid={!!shown}
        aria-describedby={statusId}
        onChange={(e) => onChange(e.target.value)}
        onKeyDown={onKeyDown}
      />
      <div className="flex min-h-6 items-center justify-between gap-3 text-xs">
        <p
          id={statusId}
          aria-live="polite"
          className={cn(
            "flex min-w-0 items-center gap-1.5",
            shown ? "text-destructive" : "text-muted-foreground",
          )}
        >
          {shown ? (
            <>
              <CircleAlert className="size-3.5 shrink-0" />
              <span className="truncate">
                Line {shown.line}, column {shown.column}: {shown.message}
              </span>
            </>
          ) : (
            value.trim() !== "" && (
              <>
                <CircleCheck className="text-success size-3.5 shrink-0" />
                Valid JSON
                {/\{\{/.test(value) && " (templates are filled in when it's sent)"}
              </>
            )
          )}
        </p>
        <Button
          type="button"
          variant="outline"
          size="xs"
          disabled={!canFormat}
          title="Format (Shift+Alt+F)"
          onMouseDown={(e) => e.preventDefault()}
          onClick={() => ref.current && format(ref.current)}
        >
          Format
        </Button>
      </div>
    </div>
  );
};
