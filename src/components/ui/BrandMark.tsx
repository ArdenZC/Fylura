import { cn } from "../../utils/tw";
import brandIcon from "../../../build/icon.png";

type BrandMarkSize = "micro" | "sidebar" | "app";

type BrandMarkProps = {
  size?: BrandMarkSize;
  decorative?: boolean;
  className?: string;
  "aria-label"?: string;
};

const sizeClasses: Record<BrandMarkSize, { root: string; image: string }> = {
  micro: {
    root: "h-5 w-5",
    image: "rounded-[5.5px] shadow-[0_0_0_1px_rgba(20,32,50,0.16),0_1px_2px_rgba(20,32,50,0.12)]"
  },
  sidebar: {
    root: "h-[30px] w-[30px]",
    image: "rounded-[8px] shadow-[0_0_0_0.5px_rgba(24,48,80,0.14),0_1px_2px_rgba(24,48,80,0.12)]"
  },
  app: {
    root: "h-20 w-20",
    image: "rounded-[18px] shadow-[0_0_0_0.5px_rgba(24,48,80,0.14),0_1px_2px_rgba(24,48,80,0.12)]"
  }
};

export function BrandMark({
  size = "sidebar",
  decorative = false,
  className,
  "aria-label": ariaLabel = "Fylura"
}: BrandMarkProps) {
  const classes = sizeClasses[size];

  return (
    <span
      className={cn("inline-block shrink-0", classes.root, className)}
      role={decorative ? undefined : "img"}
      aria-hidden={decorative || undefined}
      aria-label={decorative ? undefined : ariaLabel}
      data-brand-mark-size={size}
    >
      <img
        src={brandIcon}
        alt=""
        draggable={false}
        className={cn("block h-full w-full object-cover", classes.image)}
      />
    </span>
  );
}
