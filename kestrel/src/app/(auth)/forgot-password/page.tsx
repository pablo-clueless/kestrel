"use client";

import { zodResolver } from "@hookform/resolvers/zod";
import { ArrowLeft, Mail, MailCheck } from "lucide-react";
import Link from "next/link";
import { useState } from "react";
import { useForm } from "react-hook-form";
import { z } from "zod";

import { errorMessage, requestPasswordReset } from "@/lib/client";
import { CircleLoader } from "@/components/shared";
import { AuthField } from "@/components/shared/auth-field";
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

/** Asks for a reset link. The answer is the same whether or not the address has an account. */
const Page = () => {
  const [sentTo, setSentTo] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const {
    register,
    handleSubmit,
    formState: { errors, isSubmitting },
  } = useForm<FormValues>({ defaultValues: { email: "" }, resolver: zodResolver(schema) });

  const onSubmit = async ({ email }: FormValues) => {
    setError(null);
    try {
      await requestPasswordReset(email);
      setSentTo(email.trim());
    } catch (err) {
      setError(errorMessage(err));
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

      <form className="flex flex-col gap-3" onSubmit={handleSubmit(onSubmit)} noValidate>
        <div className="flex flex-col gap-1.5">
          <AuthField
            id="email"
            label="Email address"
            icon={Mail}
            type="email"
            autoComplete="email"
            autoFocus
            placeholder="you@example.com"
            invalid={!!errors.email}
            {...register("email")}
          />
          {errors.email && <p className="text-destructive text-xs">{errors.email.message}</p>}
        </div>

        {error && (
          <p role="alert" className="text-destructive text-sm">
            {error}
          </p>
        )}

        <Button type="submit" disabled={isSubmitting} className="mt-2 h-12 text-sm">
          {isSubmitting ? <CircleLoader radius={8} /> : "Send reset link"}
        </Button>
      </form>

      <BackToSignIn />
    </div>
  );
};

export default Page;
