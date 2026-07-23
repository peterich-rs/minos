import { cn } from "@/shared/lib/utils";
import { initials, toneClasses, type AvatarTone } from "@/shared/lib/mock-data";

type AvatarProps = {
  name: string;
  tone: AvatarTone;
  size?: "sm" | "md" | "lg";
  className?: string;
};

const sizes = {
  sm: "h-6 w-6 text-[10px]",
  md: "h-8 w-8 text-[11px]",
  lg: "h-9 w-9 text-xs",
} as const;

export function Avatar({ name, tone, size = "md", className }: AvatarProps) {
  const safeName = name?.trim() ? name : "?";
  const safeTone = toneClasses[tone] ? tone : "slate";
  return (
    <div
      className={cn(
        "inline-flex shrink-0 items-center justify-center rounded-full font-semibold",
        sizes[size],
        toneClasses[safeTone],
        className,
      )}
      aria-hidden
    >
      {initials(safeName)}
    </div>
  );
}
