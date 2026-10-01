"use client";

import { zodResolver } from "@hookform/resolvers/zod";
import { useQueryClient } from "@tanstack/react-query";
import { CircleCheck, LockKeyhole, Mail } from "lucide-react";
import Link from "next/link";
import { useRouter } from "next/navigation";
import { useEffect, useState } from "react";
import { useForm, useWatch } from "react-hook-form";
import { z } from "zod";

import { PASSWORD_MAX, PASSWORD_MESSAGE, PASSWORD_MIN } from "@/config/string";
import { authenticate, errorMessage } from "@/lib/client";
import { CircleLoader } from "@/components/shared";
import { AuthField, PasswordToggle } from "@/components/shared/auth-field";
import { Button } from "@/components/ui/button";
import { ME_KEY, useMe } from "@/hooks/use-me";
import { cn } from "cn";

const schema = z.object({
  email: z.string().trim().min(1, "Enter your email").email("Enter a valid email address"),
  password: z.string().min(PASSWORD_MIN, PASSWORD_MESSAGE).max(PASSWORD_MAX, PASSWORD_MESSAGE),
});

type FormValues = z.infer<typeof schema>;
type Mode = "login" | "signup";

const COPY: Record<Mode, { title: string; subtitle: string; action: string; tab: string }> = {
  login: {
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

const MODES: Mode[] = ["login", "signup"];

/** Sign-in and sign-up. Goes straight to the workspace when accounts are off or already signed in. */
const Page = () => {
  const router = useRouter();
  const queryClient = useQueryClient();
  const me = useMe();
  const [mode, setMode] = useState<Mode>("login");
  const [error, setError] = useState<string | null>(null);
  const [showPassword, setShowPassword] = useState(false);

  const {
    register,
    handleSubmit,
    control,
    formState: { errors, isSubmitting },
  } = useForm<FormValues>({
    defaultValues: { email: "", password: "" },
    resolver: zodResolver(schema),
  });

  const email = useWatch({ control, name: "email" });

  const proceed = me.data && (me.data.auth === "off" || me.data.user !== null);
  useEffect(() => {
    if (proceed) router.replace("/workspace");
  }, [proceed, router]);

  const onSubmit = async ({ email, password }: FormValues) => {
    setError(null);
    try {
      const signedIn = await authenticate(mode, email, password);
      queryClient.setQueryData(ME_KEY, signedIn);
      router.replace("/workspace");
    } catch (err) {
      setError(errorMessage(err));
    }
  };

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
        <p className="text-destructive">{errorMessage(me.error)}</p>
        <Button variant="outline" onClick={() => void me.refetch()}>
          Try again
        </Button>
      </div>
    );
  }

  const copy = COPY[mode];
  const canSignUp = me.data.signup;
  const emailValid = schema.shape.email.safeParse(email).success;

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
              onClick={() => {
                setError(null);
                setMode(m);
              }}
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
            trailing={
              emailValid && !errors.email ? <CircleCheck className="text-success size-4.5" /> : null
            }
            {...register("email")}
          />
          {errors.email && <p className="text-destructive text-xs">{errors.email.message}</p>}
        </div>
        <div className="flex flex-col gap-1.5">
          <AuthField
            id="password"
            label="Password"
            icon={LockKeyhole}
            type={showPassword ? "text" : "password"}
            autoComplete={mode === "signup" ? "new-password" : "current-password"}
            placeholder="••••••••"
            invalid={!!errors.password}
            aria-describedby={mode === "signup" ? "password-hint" : undefined}
            trailing={
              <PasswordToggle shown={showPassword} onToggle={() => setShowPassword((v) => !v)} />
            }
            {...register("password")}
          />
          {errors.password ? (
            <p className="text-destructive text-xs">{errors.password.message}</p>
          ) : (
            mode === "signup" && (
              <p id="password-hint" className="text-muted-foreground text-xs">
                {PASSWORD_MESSAGE}
              </p>
            )
          )}
        </div>

        {mode === "login" && me.data.mail && (
          <Link
            href="/forgot-password"
            className="text-primary -mt-1 self-end text-xs font-medium hover:underline"
          >
            Forgot password?
          </Link>
        )}

        {error && (
          <p role="alert" className="text-destructive text-sm">
            {error}
          </p>
        )}

        <Button type="submit" disabled={isSubmitting} className="mt-2 h-12 text-sm">
          {isSubmitting ? <CircleLoader radius={8} /> : copy.action}
        </Button>
      </form>
    </div>
  );
};

export default Page;
