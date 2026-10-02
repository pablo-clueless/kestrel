import type { ControllerRenderProps, FieldValues, Path, UseFormReturn } from "react-hook-form";
import { z } from "zod";
import type React from "react";

export type FormFieldType =
  | "checkbox"
  | "date"
  | "email"
  | "file"
  | "number"
  | "password"
  | "radio"
  | "search"
  | "select"
  | "text"
  | "textarea"
  | "time"
  | "toggle";

export type RangeType = "single" | "range";

export interface Option {
  label: string;
  value: string;
  disabled?: boolean;
}

export interface CustomFieldRenderProps<T extends FieldValues = FieldValues> {
  field: ControllerRenderProps<T, Path<T>>;
  form: UseFormReturn<T>;
  values: T;
  error?: string;
  disabled?: boolean;
}

export type FormFieldBase = {
  label: string;
  /** Keeps the label for screen readers but doesn't show it, for compact inline forms. */
  hideLabel?: boolean;
  /** Whether surrounding whitespace is trimmed before validating and submitting. Defaults to true,
   * except for `password` fields; set it to false on a custom field that holds a password. */
  trim?: boolean;
  className?: string;
  description?: string;
  disabled?: boolean | ((values: FieldValues) => boolean);
  error?: string;
  hidden?: boolean | ((values: FieldValues) => boolean);
  placeholder?: string;
  readOnly?: boolean | ((values: FieldValues) => boolean);
  required?: boolean | ((values: FieldValues) => boolean);
  showIf?: (values: FieldValues) => boolean;
};

export type StandardFormField = FormFieldBase & {
  type: FormFieldType;
  accept?: string;
  array?: never;
  allowCustom?: boolean | ((values: FieldValues) => boolean);
  /** Overrides the type's default, e.g. `current-password` / `new-password` for password managers. */
  autoComplete?: string;
  autoFocus?: boolean;
  custom?: never;
  dateMode?: RangeType;
  disabledDates?: Date[];
  kind?: "standard";
  max?: number;
  maxDate?: Date | ((values: FieldValues) => Date);
  maxLength?: number;
  min?: number;
  minDate?: Date | ((values: FieldValues) => Date);
  minLength?: number;
  multiple?: boolean;
  numberMode?: RangeType;
  object?: never;
  options?: Option[] | ((values: FieldValues) => Option[]);
  placeholderTo?: string;
  searchable?: boolean | ((values: FieldValues) => boolean);
  showStrength?: boolean | ((values: FieldValues) => boolean);
};

export type CustomFormField<T extends FieldValues = FieldValues> = FormFieldBase & {
  custom: true;
  customMode: "uncontrolled" | "controlled";
  render: ((props: CustomFieldRenderProps<T>) => React.ReactNode) | React.ReactNode;
  array?: never;
  kind?: "custom";
  object?: never;
  type?: never;
};

export type ObjectFormField = FormFieldBase & {
  fields: Record<string, StandardFormField>;
  kind: "object";
  array?: never;
  className?: string;
  custom?: never;
  type?: never;
};

export type PrimitiveArrayFormField = FormFieldBase & {
  itemField: StandardFormField;
  itemKind: "primitive";
  kind: "array";
  addLabel?: string;
  custom?: never;
  maxItems?: number;
  minItems?: number;
  object?: never;
  type?: never;
};

export type ObjectArrayFormField = FormFieldBase & {
  itemFields: Record<string, StandardFormField>;
  itemKind: "object";
  kind: "array";
  addLabel?: string;
  custom?: never;
  itemClassName?: string;
  maxItems?: number;
  minItems?: number;
  object?: never;
  type?: never;
};

export type ArrayFormField = PrimitiveArrayFormField | ObjectArrayFormField;

export type FormField<T extends FieldValues = FieldValues> =
  StandardFormField | CustomFormField<T> | ObjectFormField | ArrayFormField;

export interface FormSection {
  fields: string[];
  title?: string;
  description?: string;
}

export interface FormConfig<T extends FieldValues> {
  defaultValues: T;
  fields: Record<keyof T, FormField<T>>;
  onSubmit: (values: T, form: UseFormReturn<T>) => Promise<void> | void;
  schema: z.ZodType<T>;
  className?: string;
  description?: string;
  sections?: FormSection[];
  submitLabel?: string;
  title?: string;
}
