"use client";

import { zodResolver } from "@hookform/resolvers/zod";
import { useQueryClient } from "@tanstack/react-query";
import { Feather } from "lucide-react";
import { useRouter } from "next/navigation";
import { useEffect, useState } from "react";
import { useForm } from "react-hook-form";
import { z } from "zod";

import { PASSWORD_MAX, PASSWORD_MESSAGE, PASSWORD_MIN } from "@/config/string";
import { authenticate, errorMessage } from "@/lib/client";
import { CircleLoader } from "@/components/shared";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { ME_KEY, useMe } from "@/hooks/use-me";

const schema = z.object({
  email: z.string().trim().min(1, "Enter your email").email("Enter a valid email address"),
  password: z.string().min(PASSWORD_MIN, PASSWORD_MESSAGE).max(PASSWORD_MAX, PASSWORD_MESSAGE),
});

type FormValues = z.infer<typeof schema>;
type Mode = "login" | "signup";

const COPY: Record<
  Mode,
  { title: string; action: string; switchPrompt: string; switchAction: string }
> = {
  login: {
    title: "Sign in to Kestrel",
    action: "Sign in",
    switchPrompt: "New here?",
    switchAction: "Create an account",
  },
  signup: {
    title: "Create your account",
    action: "Create account",
    switchPrompt: "Already have an account?",
    switchAction: "Sign in",
  },
};

/** Sign-in and sign-up. Goes straight to the workspace when accounts are off or already signed in. */
const Page = () => {
  const router = useRouter();
  const queryClient = useQueryClient();
  const me = useMe();
  const [mode, setMode] = useState<Mode>("login");
  const [error, setError] = useState<string | null>(null);

  const {
    register,
    handleSubmit,
    formState: { errors, isSubmitting },
  } = useForm<FormValues>({
    defaultValues: { email: "", password: "" },
    resolver: zodResolver(schema),
  });

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

  return (
    <div className="flex flex-col gap-6 p-2">
      <div className="flex flex-col items-center gap-2 text-center">
        <Feather className="text-primary size-6" />
        <h1 className="text-xl font-semibold">{copy.title}</h1>
      </div>

      <form className="flex flex-col gap-4" onSubmit={handleSubmit(onSubmit)} noValidate>
        <div className="flex flex-col gap-1.5">
          <Label htmlFor="email">Email</Label>
          <Input
            id="email"
            type="email"
            autoComplete="email"
            autoFocus
            aria-invalid={!!errors.email}
            {...register("email")}
          />
          {errors.email && <p className="text-destructive text-xs">{errors.email.message}</p>}
        </div>
        <div className="flex flex-col gap-1.5">
          <Label htmlFor="password">Password</Label>
          <Input
            id="password"
            type="password"
            autoComplete={mode === "signup" ? "new-password" : "current-password"}
            aria-invalid={!!errors.password}
            aria-describedby={mode === "signup" ? "password-hint" : undefined}
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

        {error && (
          <p role="alert" className="text-destructive text-sm">
            {error}
          </p>
        )}

        <Button type="submit" disabled={isSubmitting}>
          {isSubmitting ? <CircleLoader radius={8} /> : copy.action}
        </Button>
      </form>

      {canSignUp && (
        <p className="text-muted-foreground text-center text-sm">
          {copy.switchPrompt}{" "}
          <button
            type="button"
            className="text-primary font-medium hover:underline"
            onClick={() => {
              setError(null);
              setMode(mode === "login" ? "signup" : "login");
            }}
          >
            {copy.switchAction}
          </button>
        </p>
      )}
    </div>
  );
};

export default Page;
