"use client";

import { useEffect, useState } from "react";

import { Button } from "../ui/button";

type Props = Omit<React.ComponentProps<typeof Button>, "onClick" | "children"> & {
  children: React.ReactNode;
  onConfirm: () => void;
  pending?: boolean;
  /** Shown while it's waiting for the second click. */
  confirmLabel?: React.ReactNode;
  pendingLabel?: React.ReactNode;
};

/** A button that needs a second click within a few seconds, for actions that can't be undone. It
 * disarms itself after that, so a stray click later doesn't do anything. */
export const ConfirmButton = ({
  children,
  onConfirm,
  pending = false,
  confirmLabel = "Click again to confirm",
  pendingLabel,
  disabled,
  ...props
}: Props) => {
  const [armed, setArmed] = useState(false);
  useEffect(() => {
    if (!armed) return;
    const t = setTimeout(() => setArmed(false), 5000);
    return () => clearTimeout(t);
  }, [armed]);

  return (
    <Button
      {...props}
      disabled={disabled || pending}
      onClick={() => {
        if (!armed) return setArmed(true);
        setArmed(false);
        onConfirm();
      }}
    >
      {pending ? (pendingLabel ?? children) : armed ? confirmLabel : children}
    </Button>
  );
};
