import Image from "next/image";
import React from "react";

interface Props {
  children: React.ReactNode;
}

const IMAGE = "/assets/logo.png";
const ICON = "/assets/icon.png";

export default function AuthLayout({ children }: Props) {
  return (
    <div className="bg-card grid min-h-dvh w-full grid-cols-1 lg:h-dvh lg:grid-cols-2 lg:overflow-hidden">
      <div className="flex min-h-dvh flex-col px-6 py-10 lg:min-h-0">
        <div className="flex items-center justify-center gap-2 text-lg font-semibold">
          <div className="relative aspect-[4.2/1] w-1/3">
            <Image alt="Kestrel" className="" fill src={IMAGE} />
          </div>
        </div>
        <main className="flex flex-1 items-center justify-center py-10">
          <div className="w-full max-w-sm">{children}</div>
        </main>
        <p className="text-muted-foreground mx-auto max-w-md text-center text-xs leading-relaxed">
          &copy;{new Date().getFullYear()}. All rights reserved.
        </p>
      </div>
      <div
        aria-hidden
        className="bg-auth bg-primary relative hidden h-full min-h-dvh place-items-center overflow-hidden bg-cover bg-center py-10 bg-blend-luminosity lg:grid lg:border-l"
      >
        <div className="absolute bottom-4 left-6 flex items-center gap-x-4">
          <div className="relative aspect-square w-7 rounded-full bg-white p-1">
            <Image alt="Kestrel" className="p-1" fill src={ICON} />
          </div>
          <p className="text-primary mx-auto max-w-md text-center text-xs leading-relaxed">
            <b>Kestrel</b> by Samson Okunola.
          </p>
        </div>
      </div>
    </div>
  );
}
