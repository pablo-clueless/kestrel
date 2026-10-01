import { useState } from "react";

import { Button } from "../ui/button";
import { cn } from "cn";
import {
  Popover,
  PopoverContent,
  PopoverDescription,
  PopoverHeader,
  PopoverTitle,
  PopoverTrigger,
} from "../ui/popover";

type Variant = "default" | "destructive" | "info" | "success" | "warning";

const VARIANTS: Record<Variant, string> = {
  default: "hover:bg-gray-100 text-gray-700 dark:hover:bg-gray-950 dark:text-gray-400",
  destructive: "hover:bg-red-50 text-red-700 dark:hover:bg-red-950 dark:text-red-400",
  info: "hover:bg-blue-50 text-blue-700 dark:hover:bg-blue-950 dark:text-blue-400",
  success: "hover:bg-green-50 text-green-700 dark:hover:bg-green-950 dark:text-green-400",
  warning: "hover:bg-amber-50 text-amber-700 dark:bg-amber-950 dark:text-amber-400",
};

const menu: { id: string; label: string; href: string; variant: Variant }[] = [
  { id: "profile", label: "Profile", href: "/profile", variant: "default" },
  { id: "settings", label: "Settings", href: "/settings", variant: "default" },
  { id: "logout", label: "Logout", href: "/signout", variant: "destructive" },
];

export const LoggUser = () => {
  const [open, setOpen] = useState(false);

  return (
    <Popover onOpenChange={setOpen} open={open}>
      <PopoverTrigger
        render={
          <Button
            onClick={() => setOpen}
            variant="outline"
            size="icon"
            className="w-50"
            aria-label="Toggle theme"
          ></Button>
        }
      />
      <PopoverContent align="end">
        <PopoverHeader>
          <PopoverTitle></PopoverTitle>
          <PopoverDescription></PopoverDescription>
        </PopoverHeader>
        <div className="">
          {menu.map((item) => (
            <div
              className={cn(
                "cursor-pointer border-b px-2.5 py-1.5 text-sm last:border-b-0",
                VARIANTS[item.variant],
              )}
              key={item.id}
            >
              <span>{item.label}</span>
            </div>
          ))}
        </div>
      </PopoverContent>
    </Popover>
  );
};
