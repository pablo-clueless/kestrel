"use client";

import { CircleCheck, LockKeyhole, Mail, ShieldCheck } from "lucide-react";
import { useQueryClient } from "@tanstack/react-query";
import { useRouter } from "next/navigation";
import { useEffect, useState } from "react";
import { toast } from "sonner";
import Link from "next/link";
import { z } from "zod";

import { PASSWORD_MAX, PASSWORD_MESSAGE, PASSWORD_MIN } from "@/config/string";
import { AuthControl, PasswordToggle } from "@/components/shared/auth-field";
import { authenticate, completeTwoFactor, errorMessage } from "@/lib/client";
import type { MeResponse } from "@/types/engine/MeResponse";
import { Form, type FormField } from "@/components/form";
import { CircleLoader } from "@/components/shared";
import { Button } from "@/components/ui/button";
import { ME_KEY, useMe } from "@/hooks/use-me";
import { cn } from "cn";

const schema = z.object({
  email: z.string().trim().min(1, "Enter your email").email("Enter a valid email address"),
  password: z.string().min(PASSWORD_MIN, PASSWORD_MESSAGE).max(PASSWORD_MAX, PASSWORD_MESSAGE),
});

type FormValues = z.infer<typeof schema>;
type Mode = "signin" | "signup";

const COPY: Record<Mode, { title: string; subtitle: string; action: string; tab: string }> = {
  signin: {
    title: "Welcome back",
    subtitle: "Sign in with your email and password.",
    action: "Sign in",
    tab: "Sign in",
  },
  signup: {
    title: "Create your account",
    subtitle: "Your workspace follows you to any device.",
    action: "Create account",
    tab: "Sign up",
  },
};

const defaultValues: FormValues = { email: "", password: "" };

const MODES: Mode[] = ["signin", "signup"];

/** Where to go once signed in: `?next=` (e.g. back to an invite) if it's a path on this site, else the
 * workspace. Only same-site paths, so a crafted link can't send people elsewhere after signing in. */
const nextPath = () => {
  const next = new URLSearchParams(window.location.search).get("next");
  return next && /^\/(?![/\\])/.test(next) ? next : "/workspace";
};

/** Sign-in and sign-up. Goes straight to the workspace when accounts are off or already signed in. */
const Page = () => {
  const [showPassword, setShowPassword] = useState(false);
  const [mode, setMode] = useState<Mode>("signin");
  /** Set once the password is right on an account with two-factor on: the code step follows. */
  const [challenge, setChallenge] = useState<string | null>(null);
  const queryClient = useQueryClient();
  const router = useRouter();
  const me = useMe();

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
          trailing={
            !error && schema.shape.email.safeParse(field.value).success ? (
              <CircleCheck className="text-success size-4.5" />
            ) : null
          }
        />
      ),
    },
    password: {
      label: "Password",
      trim: false,
      custom: true,
      customMode: "controlled",
      render: ({ field, error }) => (
        <AuthControl
          field={field}
          error={error}
          hint={mode === "signup" ? PASSWORD_MESSAGE : undefined}
          id="password"
          label="Password"
          icon={LockKeyhole}
          type={showPassword ? "text" : "password"}
          autoComplete={mode === "signup" ? "new-password" : "current-password"}
          placeholder="••••••••"
          trailing={
            <PasswordToggle shown={showPassword} onToggle={() => setShowPassword((v) => !v)} />
          }
        />
      ),
    },
  };

  const proceed = me.data && (me.data.auth === "off" || me.data.user !== null);
  useEffect(() => {
    if (me.isError) toast.error(errorMessage(me.error));
  }, [me.isError, me.error]);
  useEffect(() => {
    if (proceed) router.replace(nextPath());
  }, [proceed, router]);

  const onSubmit = async ({ email, password }: FormValues) => {
    try {
      const res = await authenticate(mode, email, password);
      if ("twoFactorChallenge" in res) {
        setChallenge(res.twoFactorChallenge);
        return;
      }
      queryClient.setQueryData(ME_KEY, res);
      toast.success("Signed in successfully");
      router.replace(nextPath());
    } catch (err) {
      toast.error(errorMessage(err));
    }
  };

  if (challenge) {
    return (
      <TwoFactorStep
        challenge={challenge}
        onBack={() => setChallenge(null)}
        onSignedIn={(me) => {
          queryClient.setQueryData(ME_KEY, me);
          toast.success("Signed in successfully");
          router.replace(nextPath());
        }}
      />
    );
  }

  if (me.isPending || proceed) {
    return (
      <div className="grid h-40 place-items-center">
        <CircleLoader />
      </div>
    );
  }

  if (me.isError) {
    return (
      <div className="flex flex-col items-center gap-3 p-4 text-center text-sm">
        <p className="text-muted-foreground">Couldn&apos;t check whether you&apos;re signed in.</p>
        <Button variant="outline" onClick={() => void me.refetch()}>
          Try again
        </Button>
      </div>
    );
  }

  const copy = COPY[mode];
  const canSignUp = me.data.signup;

  return (
    <div className="flex flex-col gap-6">
      <div className="flex flex-col items-center gap-1.5 text-center">
        <h1 className="text-3xl font-bold tracking-tight">{copy.title}</h1>
        <p className="text-muted-foreground text-sm">{copy.subtitle}</p>
      </div>
      {canSignUp && (
        <div role="tablist" aria-label="Account" className="bg-muted grid grid-cols-2 p-1">
          {MODES.map((m) => (
            <button
              key={m}
              type="button"
              role="tab"
              aria-selected={mode === m}
              onClick={() => setMode(m)}
              className={cn(
                "h-9 text-sm transition-colors",
                mode === m
                  ? "bg-card text-foreground font-medium shadow-sm"
                  : "text-muted-foreground hover:text-foreground",
              )}
            >
              {COPY[m].tab}
            </button>
          ))}
        </div>
      )}
      <Form
        schema={schema}
        defaultValues={defaultValues}
        fields={fields}
        onSubmit={onSubmit}
        toastOnInvalid={false}
      >
        {({ field, isSubmitting }) => (
          <div className="flex flex-col gap-3">
            {field("email")}
            {field("password")}
            {mode === "signin" && me.data.mail && (
              <Link
                href="/forgot-password"
                className="text-primary -mt-1 self-end text-xs font-medium hover:underline"
              >
                Forgot password?
              </Link>
            )}
            <Button type="submit" disabled={isSubmitting} className="mt-2 h-12 text-sm">
              {isSubmitting ? <CircleLoader radius={8} /> : copy.action}
            </Button>
          </div>
        )}
      </Form>
    </div>
  );
};

const codeSchema = z.object({
  code: z.string().trim().min(6, "Enter the 6-digit code, or a recovery code"),
});

/** The second step for accounts with two-factor on: a code from the authenticator app, or one of the
 * recovery codes. The challenge lasts 5 minutes and 5 tries; after that, back to the password. */
const TwoFactorStep = ({
  challenge,
  onBack,
  onSignedIn,
}: {
  challenge: string;
  onBack: () => void;
  onSignedIn: (me: MeResponse) => void;
}) => (
  <div className="flex flex-col gap-6">
    <div className="flex flex-col items-center gap-6 text-center">
      <span className="bg-primary/10 text-primary grid size-14 place-items-center rounded-2xl">
        <ShieldCheck className="size-7" />
      </span>
      <div className="flex flex-col gap-1.5">
        <h1 className="text-3xl font-bold tracking-tight">Two-factor sign-in</h1>
        <p className="text-muted-foreground text-sm">
          Enter the code from your authenticator app. Lost your phone? Use one of your recovery
          codes instead.
        </p>
      </div>
    </div>
    <Form
      schema={codeSchema}
      defaultValues={{ code: "" }}
      toastOnInvalid={false}
      fields={{
        code: {
          type: "text",
          label: "Code",
          autoComplete: "one-time-code",
          autoFocus: true,
          placeholder: "123456",
        },
      }}
      onSubmit={async ({ code }, form) => {
        try {
          onSignedIn(await completeTwoFactor(challenge, code));
        } catch (err) {
          const message = errorMessage(err);
          // Out of tries or out of time: the challenge is gone, so start over.
          if (/password again/i.test(message)) {
            toast.error(message);
            onBack();
          } else {
            form.setError("code", { message });
          }
        }
      }}
    >
      {({ field, isSubmitting }) => (
        <div className="flex flex-col gap-3">
          {field("code")}
          <Button type="submit" disabled={isSubmitting} className="mt-2 h-12 text-sm">
            {isSubmitting ? <CircleLoader radius={8} /> : "Verify"}
          </Button>
          <button
            type="button"
            onClick={onBack}
            className="text-muted-foreground hover:text-foreground self-center text-xs"
          >
            Use a different account
          </button>
        </div>
      )}
    </Form>
  </div>
);

export default Page;
