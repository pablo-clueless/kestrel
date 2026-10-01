"use client";

import { AnimatePresence, motion } from "framer-motion";
import React from "react";

import { cn } from "cn";

interface Props extends React.HTMLAttributes<HTMLDivElement> {
  selected: string;
  value: string;
  children?: React.ReactNode;
  className?: string;
  innerClassName?: string;
  animated?: boolean;
  preserveContent?: boolean;
}

const TabPanel = React.forwardRef<HTMLDivElement, Props>(
  (
    {
      selected,
      value,
      children,
      className,
      innerClassName,
      animated = false,
      preserveContent = false,
    },
    ref,
  ) => {
    if (typeof selected !== typeof value) {
      throw new Error("TabPanel: selected and value must be of the same type");
    }

    const isSelected = selected === value;
    const content = <div className={cn("h-full w-full", innerClassName)}>{children}</div>;

    return (
      <div
        ref={ref}
        role="tabpanel"
        hidden={!isSelected}
        id={`tabpanel-${value}`}
        aria-labelledby={`tab-${value}`}
        aria-hidden={!isSelected}
        tabIndex={isSelected ? 0 : -1}
        className={cn("h-full w-full", className)}
      >
        {animated ? (
          <AnimatePresence mode="wait">
            {(isSelected || preserveContent) && (
              <motion.div
                key={`${value}`}
                initial={{ opacity: 0, y: 10 }}
                animate={{ opacity: 1, y: 0 }}
                exit={{ opacity: 0, y: -10 }}
                transition={{ duration: 0.2 }}
                className="h-full w-full"
              >
                {content}
              </motion.div>
            )}
          </AnimatePresence>
        ) : (
          isSelected && content
        )}
      </div>
    );
  },
);

TabPanel.displayName = "TabPanel";

export { TabPanel };
export type { Props as TabPanelProps };
