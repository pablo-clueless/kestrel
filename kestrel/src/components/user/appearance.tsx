"use client";

import { Monitor, Moon, Sun, type LucideIcon } from "lucide-react";
import { useTheme } from "next-themes";
import { useSyncExternalStore } from "react";

import { TabPanel } from "../shared";
import { cn } from "cn";

interface Props {
  selected: string;
}

const THEMES: { id: string; label: string; icon: LucideIcon; hint: string }[] = [
  { id: "light", label: "Light", icon: Sun, hint: "Always light." },
  { id: "dark", label: "Dark", icon: Moon, hint: "Always dark." },
  { id: "system", label: "System", icon: Monitor, hint: "Follows your device's setting." },
];

/** False while prerendering and hydrating: the theme lives in this browser, so the server can't know it. */
const useMounted = () =>
  useSyncExternalStore(
    () => () => {},
    () => true,
    () => false,
  );

export const Appearance = ({ selected }: Props) => {
  const { theme, setTheme } = useTheme();
  const mounted = useMounted();

  return (
    <TabPanel selected={selected} value="appearance">
      <div className="bg-background flex h-[calc(100%-36px)] flex-col gap-4 overflow-y-auto p-5">
        <section className="flex flex-col gap-3">
          <div>
            <h2 className="text-sm font-semibold">Theme</h2>
            <p className="text-muted-foreground text-sm">
              Saved in this browser, so each device keeps its own.
            </p>
          </div>
          <div role="radiogroup" aria-label="Theme" className="grid max-w-lg grid-cols-3 gap-2">
            {THEMES.map(({ id, label, icon: Icon, hint }) => {
              const active = mounted && theme === id;
              return (
                <button
                  key={id}
                  type="button"
                  role="radio"
                  aria-checked={active}
                  onClick={() => setTheme(id)}
                  className={cn(
                    "flex flex-col items-start gap-2 rounded-xs border p-3 text-left text-sm transition-colors",
                    active ? "border-primary bg-primary/5" : "hover:bg-muted/60",
                  )}
                >
                  <Icon
                    className={cn("size-4", active ? "text-primary" : "text-muted-foreground")}
                  />
                  <span className="font-medium">{label}</span>
                  <span className="text-muted-foreground text-xs">{hint}</span>
                </button>
              );
            })}
          </div>
        </section>
      </div>
    </TabPanel>
  );
};
