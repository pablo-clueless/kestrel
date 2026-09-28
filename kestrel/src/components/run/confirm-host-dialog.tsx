"use client";

import { useState } from "react";

import { confirmHost, errorMessage } from "@/lib/client";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";

interface Props {
  /** The host awaiting confirmation; the dialog is open while this is set. */
  host: string | null;
  onClose: () => void;
  /** Called after the engine recorded the confirmation. */
  onConfirmed: () => void;
}

/** Load tests against anything that isn't this machine need an explicit, per-host confirmation. */
export const ConfirmHostDialog = ({ host, onClose, onConfirmed }: Props) => {
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const confirm = async () => {
    if (!host) return;
    setPending(true);
    setError(null);
    try {
      await confirmHost(host);
      onConfirmed();
    } catch (err) {
      setError(errorMessage(err));
    } finally {
      setPending(false);
    }
  };

  return (
    <Dialog open={host !== null} onOpenChange={(open) => !open && onClose()}>
      <DialogContent className="sm:max-w-150">
        <DialogHeader>
          <DialogTitle>Load test {host}?</DialogTitle>
          <DialogDescription>
            This sends sustained traffic to <span className="font-mono">{host}</span>. Only continue
            if you own it or are authorised to test it. Confirmation lasts until the engine
            restarts.
          </DialogDescription>
        </DialogHeader>
        {error && <p className="text-sm text-red-600">{error}</p>}
        <DialogFooter>
          <Button variant="outline" onClick={onClose}>
            Cancel
          </Button>
          <Button onClick={confirm} disabled={pending}>
            I own or am authorised to test this host
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
};
