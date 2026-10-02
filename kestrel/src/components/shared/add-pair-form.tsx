"use client";

import { Plus } from "lucide-react";
import { z } from "zod";

import { Form } from "@/components/form";
import { cn } from "cn";

const schema = z.object({ key: z.string(), value: z.string() });

interface Props {
  onAdd: (key: string, value: string) => void;
  /** E.g. "Variable" or "Secret": names the fields for screen readers. */
  noun: string;
  keyPlaceholder?: string;
  valuePlaceholder?: string;
  /** A write-only value, typed into a password field. */
  secret?: boolean;
  /** Width of the name field, to line up with the rows above it. */
  keyClassName?: string;
}

/**
 * The "add" row under a list of variables or secrets: a name, a value and a + button. An empty
 * name does nothing. The name is trimmed; the value is kept exactly as typed, since it's a template
 * (or a secret) sent as written.
 */
export const AddPairForm = ({
  onAdd,
  noun,
  keyPlaceholder = "name",
  valuePlaceholder = "value",
  secret,
  keyClassName = "w-16",
}: Props) => (
  <Form
    schema={schema}
    defaultValues={{ key: "", value: "" }}
    fields={{
      key: {
        type: "text",
        label: `${noun} name`,
        hideLabel: true,
        placeholder: keyPlaceholder,
        autoComplete: "off",
        className: cn("shrink-0", keyClassName),
      },
      value: {
        type: secret ? "password" : "text",
        label: `${noun} value`,
        hideLabel: true,
        placeholder: valuePlaceholder,
        autoComplete: "off",
        trim: false,
        className: "flex-1 [&_input]:font-mono",
      },
    }}
    onSubmit={({ key, value }, form) => {
      if (!key) return;
      onAdd(key, value);
      form.reset();
    }}
  >
    {({ field }) => (
      <div className="flex items-center gap-3">
        {field("key")}
        {field("value")}
        <button
          type="submit"
          className="text-muted-foreground shrink-0 hover:text-green-500"
          aria-label={`Add ${noun.toLowerCase()}`}
        >
          <Plus className="size-4" />
        </button>
      </div>
    )}
  </Form>
);
