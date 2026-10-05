"use client";

import { ArrowLeft, Mail, MailCheck } from "lucide-react";
import { useState } from "react";
import { toast } from "sonner";
import Link from "next/link";
import { z } from "zod";

import { errorMessage, requestPasswordReset } from "@/lib/client";
import { AuthControl } from "@/components/shared/auth-field";
import { Form, type FormField } from "@/components/form";
import { CircleLoader } from "@/components/shared";
import { Button } from "@/components/ui/button";

const schema = z.object({
  email: z.string().trim().min(1, "Enter your email").email("Enter a valid email address"),
});

type FormValues = z.infer<typeof schema>;

const BackToSignIn = () => (
  <Link
    href="/"
    className="text-muted-foreground hover:text-foreground inline-flex items-center justify-center gap-1.5 text-sm"
  >
    <ArrowLeft className="size-4" /> Back to sign in
  </Link>
);

const fields: Record<keyof FormValues, FormField<FormValues>> = {
  email: {
    label: "Email address",
    custom: true,
    customMode: "controlled",
    render: ({ field, error }) => (
      <AuthControl
        field={field}
        error={error}
        id="email"
        label="Email address"
        icon={Mail}
        type="email"
        autoComplete="email"
        autoFocus
        placeholder="you@example.com"
      />
    ),
  },
};

/** Asks for a reset link. The answer is the same whether or not the address has an account. */
const Page = () => {
  const [sentTo, setSentTo] = useState<string | null>(null);

  const onSubmit = async ({ email }: FormValues) => {
    try {
      await requestPasswordReset(email);
      setSentTo(email);
    } catch (err) {
      toast.error(errorMessage(err));
    }
  };

  if (sentTo) {
    return (
      <div className="flex flex-col items-center gap-6 text-center">
        <span className="bg-accent text-accent-foreground grid size-14 place-items-center rounded-2xl">
          <MailCheck className="size-7" />
        </span>
        <div className="flex flex-col gap-1.5">
          <h1 className="text-3xl font-bold tracking-tight">Check your email</h1>
          <p className="text-muted-foreground text-sm">
            If <span className="text-foreground font-medium">{sentTo}</span> has a Kestrel account,
            a link to reset its password is on its way. It works once, for 30 minutes.
          </p>
        </div>
        <BackToSignIn />
      </div>
    );
  }

  return (
    <div className="flex flex-col gap-6">
      <div className="flex flex-col items-center gap-1.5 text-center">
        <h1 className="text-3xl font-bold tracking-tight">Forgot your password?</h1>
        <p className="text-muted-foreground text-sm">
          Enter your email and we&apos;ll send you a link to choose a new one.
        </p>
      </div>

      <Form
        schema={schema}
        defaultValues={{ email: "" }}
        fields={fields}
        onSubmit={onSubmit}
        toastOnInvalid={false}
      >
        {({ field, isSubmitting }) => (
          <div className="flex flex-col gap-3">
            {field("email")}
            <Button type="submit" disabled={isSubmitting} className="mt-2 h-12 text-sm">
              {isSubmitting ? <CircleLoader radius={8} /> : "Send reset link"}
            </Button>
          </div>
        )}
      </Form>

      <BackToSignIn />
    </div>
  );
};

export default Page;
