"use client";

import { useRouter } from "next/navigation";
import { useEffect } from "react";

import { errorMessage } from "@/lib/client";
import { CircleLoader } from "./loader";
import { useMe } from "@/hooks/use-me";
import { Button } from "../ui/button";

/**
 * Renders the app only once it's known who's signed in, so nothing loads workspace data before
 * `/auth/me` has said which workspace to use. Signed out (accounts on) → the sign-in page. This is
 * for the experience only: the engine enforces sessions on every call.
 */
export const AuthGate = ({ children }: { children: React.ReactNode }) => {
  const router = useRouter();
  const me = useMe();
  const signedOut = me.data?.auth === "on" && me.data.user === null;

  useEffect(() => {
    if (signedOut) router.replace("/");
  }, [signedOut, router]);

  if (me.isError) {
    return (
      <div className="flex h-dvh flex-col items-center justify-center gap-3 p-4 text-center text-sm">
        <p className="text-destructive">{errorMessage(me.error)}</p>
        <Button variant="outline" onClick={() => void me.refetch()}>
          Try again
        </Button>
      </div>
    );
  }
  if (me.isPending || signedOut) {
    return (
      <div className="grid h-dvh place-items-center" aria-busy="true" aria-label="Loading">
        <CircleLoader />
      </div>
    );
  }
  return children;
};
