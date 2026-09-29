import { motion } from "framer-motion";
import { cn } from "cn";

// CSS color values for Framer Motion color interpolation
const CIRCLE_COLORS = [
  "#86efac", // green-300
  "#fca5a5", // red-300
  "#fde047", // yellow-300
  "#5eead4", // teal-300
  "#fdba74", // orange-300
  "#93c5fd", // blue-300
  "#c084fc", // purple-300
  "#22d3ee", // cyan-300
  "#f472b6", // pink-300
  "#a5b4fc", // indigo-300
];

const BAR_COLORS = [
  "bg-green-300",
  "bg-red-300",
  "bg-yellow-300",
  "bg-teal-300",
  "bg-orange-300",
  "bg-blue-300",
  "bg-purple-300",
  "bg-cyan-300",
  "bg-pink-300",
  "bg-indigo-300",
];

interface BarLoaderProps {
  className?: string;
}

/** colors race after one another horizontally in an infinite loop */
export const BarLoader = ({ className }: BarLoaderProps) => {
  const DURATION = 2.5;

  return (
    <motion.div
      className={cn(
        "relative h-1 w-full overflow-hidden rounded-md bg-black/5 dark:bg-white/10",
        className,
      )}
    >
      {BAR_COLORS.map((color, i) => (
        <motion.div
          key={color}
          className={cn("absolute inset-y-0 left-0 w-full rounded-md", color)}
          initial={{ x: "-100%" }}
          animate={{ x: "100%" }}
          transition={{
            duration: DURATION,
            repeat: Infinity,
            ease: "linear",
            delay: (DURATION / BAR_COLORS.length) * i,
          }}
        />
      ))}
    </motion.div>
  );
};

interface CircleLoaderProps {
  className?: string;
  radius?: number;
}

/** google-styled circle-loop loader */
export const CircleLoader = ({ className, radius = 16 }: CircleLoaderProps) => {
  const borderWidth = Math.max(2, radius / 5);

  return (
    <motion.div
      className={cn("aspect-square rounded-full", className)}
      style={{ width: radius * 2, height: radius * 2 }}
      animate={{ rotate: 360 }}
      transition={{ duration: 1.1, repeat: Infinity, ease: "linear" }}
    >
      <motion.div
        className="box-border h-full w-full rounded-full border-2 border-solid border-transparent"
        style={{ borderWidth }}
        animate={{ borderTopColor: [...CIRCLE_COLORS, CIRCLE_COLORS[0]] }}
        transition={{
          duration: CIRCLE_COLORS.length * 0.6,
          repeat: Infinity,
          ease: "linear",
        }}
      />
    </motion.div>
  );
};
