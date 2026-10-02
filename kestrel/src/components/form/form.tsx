import type { FieldValues, Resolver, DefaultValues, Path } from "react-hook-form";
import { zodResolver } from "@hookform/resolvers/zod";
import { useForm } from "react-hook-form";
import { useMemo } from "react";
import type React from "react";
import { toast } from "sonner";

import type { FormConfig, FormField } from "./type";
import { Field } from "./field";
import { cn } from "cn";
import {
  firstFieldErrorMessage,
  isObjectArrayField,
  isObjectField,
  isPrimitiveArrayField,
  trimDeep,
} from "./utils";

type UseFormReturn<T extends FieldValues> = ReturnType<typeof useForm<T>>;
type ZodType<Output, Input extends FieldValues = FieldValues> = {
  _output: Output;
  _input: Input;
  _def: { typeName: string };
};

export const Form = <T extends FieldValues>(
  config: Omit<FormConfig<T>, "sections" | "title" | "description" | "submitLabel"> & {
    children: (renderProps: {
      field: (name: Path<T>) => React.ReactNode;
      form: UseFormReturn<T>;
      isDirty: boolean;
      isSubmitting: boolean;
    }) => React.ReactNode;
    className?: string;
    disabled?: boolean;
    mode?: "all" | "onBlur" | "onChange" | "onSubmit" | "onTouched";
    /** Toast the first error when submitting an invalid form (default). Turn it off when every field
     * shows its own error inline, so the message doesn't appear twice. */
    toastOnInvalid?: boolean;
  },
) => {
  const {
    children,
    defaultValues,
    fields,
    onSubmit,
    schema,
    className,
    disabled,
    mode,
    toastOnInvalid = true,
  } = config;

  // Fields whose values are submitted exactly as typed: passwords, and any marked `trim: false`.
  const passwords = useMemo(() => {
    const keys = new Set<string>();
    for (const [key, def] of Object.entries(fields ?? {})) {
      const { type, trim } = (def ?? {}) as { type?: string; trim?: boolean };
      if (type === "password" || trim === false) keys.add(key);
    }
    return keys;
  }, [fields]);

  const resolver = useMemo<Resolver<T>>(() => {
    const base = zodResolver(schema as unknown as ZodType<T>, undefined, {
      raw: false,
    }) as Resolver<T>;
    return (values, context, options) => base(trimDeep(values, passwords), context, options);
  }, [schema, passwords]);

  const form = useForm<T>({
    resolver,
    defaultValues: defaultValues as unknown as DefaultValues<T>,
    mode,
  });

  const {
    handleSubmit,
    formState: { isDirty, isSubmitting },
  } = form;

  const resolveField = (name: Path<T>): FormField<T> | undefined => {
    const [head, ...rest] = String(name).split(".");
    let current: FormField<T> | undefined = (fields as Record<string, FormField<T>>)[head];
    for (const part of rest) {
      if (!current) return undefined;
      const isIndex = /^\d+$/.test(part);
      if (isObjectField(current)) current = current.fields[part];
      else if (isObjectArrayField(current)) current = isIndex ? current : current.itemFields[part];
      else if (isPrimitiveArrayField(current)) current = isIndex ? current.itemField : undefined;
      else return undefined;
    }
    return current;
  };

  const field = (name: Path<T>) => {
    const field = resolveField(name);
    if (!field) return null;
    return (
      <Field
        field={field}
        form={form}
        name={String(name)}
        className={field.className}
        disabled={isSubmitting || disabled}
      />
    );
  };

  const submit = (event: React.SubmitEvent<HTMLFormElement>) => {
    event.preventDefault();
    event.stopPropagation();
    void handleSubmit(
      async (values) => {
        if (disabled) return;
        const cleaned = trimDeep(values, passwords) as unknown as T;
        await onSubmit(cleaned, form);
      },
      (errors) => {
        if (!toastOnInvalid) return;
        toast.error(firstFieldErrorMessage(errors) ?? "Please fix the highlighted fields.");
      },
    )(event);
  };

  return (
    <form action="#" className={cn("w-full", className)} method="post" noValidate onSubmit={submit}>
      {children({ field, isSubmitting, isDirty, form })}
    </form>
  );
};
