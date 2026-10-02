"use client";

import { z } from "zod";

import { Form } from "@/components/form";
import { Input } from "@/components/ui/input";

const schema = z.object({ name: z.string() });

interface Props {
  /** Also the input's accessible name. */
  placeholder: string;
  onSubmit: (name: string) => void;
  onCancel: () => void;
  className?: string;
}

/**
 * A one-field form for naming something new in the sidebar (a collection, a group). Enter or
 * clicking away creates it; Escape, or leaving it empty, cancels.
 */
export const InlineNameForm = ({ placeholder, onSubmit, onCancel, className }: Props) => (
  <Form
    className={className}
    schema={schema}
    defaultValues={{ name: "" }}
    toastOnInvalid={false}
    onSubmit={({ name }) => (name ? onSubmit(name) : onCancel())}
    fields={{
      name: {
        label: placeholder,
        custom: true,
        customMode: "controlled",
        render: ({ field }) => (
          <Input
            ref={field.ref}
            name={field.name}
            autoFocus
            aria-label={placeholder}
            placeholder={placeholder}
            value={field.value}
            onChange={(e) => field.onChange(e.target.value)}
            onBlur={(e) => {
              field.onBlur();
              e.currentTarget.form?.requestSubmit();
            }}
            onKeyDown={(e) => e.key === "Escape" && onCancel()}
          />
        ),
      },
    }}
  >
    {({ field }) => field("name")}
  </Form>
);
