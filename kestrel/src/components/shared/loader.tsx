import { motion } from "framer-motion";
import { cn } from "cn";

const COLORS = [
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
      {COLORS.map((color, i) => (
        <motion.div
          key={color}
          className={cn("absolute inset-y-0 left-0 w-full rounded-md", color)}
          initial={{ x: "-100%" }}
          animate={{ x: "100%" }}
          transition={{
            duration: DURATION,
            repeat: Infinity,
            ease: "linear",
            delay: (DURATION / COLORS.length) * i, // staggered chase
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
export const CircleLoader = ({ className, radius = 20 }: CircleLoaderProps) => {
  return (
    <motion.div
      className={cn("aspect-square rounded-full", className)}
      style={{ width: radius * 2 }}
      animate={{ rotate: 360 }}
      transition={{ duration: 1.1, repeat: Infinity, ease: "linear" }}
    >
      <motion.div
        className="box-border h-full w-full rounded-full border-solid border-transparent"
        style={{ borderWidth: Math.max(2, radius / 5) }}
        animate={{ borderTopColor: [...COLORS, COLORS[0]] }}
        transition={{ duration: COLORS.length * 0.6, repeat: Infinity, ease: "linear" }}
      />
    </motion.div>
  );
};
