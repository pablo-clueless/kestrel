import { PanelLeft } from "lucide-react";

import { EnvironmentBar } from "../workspace";

export const Header = () => {
  return (
    <header className="mx-auto flex h-16 w-full items-center border-b">
      <div className="flex w-full items-center justify-between px-4">
        <div className="flex items-center gap-3">
          <button>
            <PanelLeft className="size-4" />
          </button>
        </div>
        <EnvironmentBar />
      </div>
    </header>
  );
};
