import React from "react";

import { cn } from "cn";

interface Props {
  title: string;
  children?: React.ReactNode;
  className?: string;
}

export const Card = ({ title, children, className }: Props) => {
  return (
    <div className={cn("min-h-0 border", className)}>
      <div className="flex h-9 items-center justify-between px-4">
        <p className="font-medium">{title}</p>
      </div>
      <div className="p-4">{children}</div>
    </div>
  );
};
