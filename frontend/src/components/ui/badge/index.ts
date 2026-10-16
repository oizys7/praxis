import { cva, type VariantProps } from "class-variance-authority";

export { default as Badge } from "./Badge.vue";

/** Badge 样式变体 */
export const badgeVariants = cva(
  "inline-flex items-center rounded-md border px-2.5 py-0.5 text-xs font-semibold transition-colors",
  {
    variants: {
      variant: {
        /* 语义归拢三色：书绿成长 / 墨蓝进行 / 朱批警示，其余一律沙框中性 */
        default: "border-border bg-secondary text-secondary-foreground",
        secondary: "border-border bg-secondary text-secondary-foreground",
        outline: "border-border text-foreground",
        success: "border-transparent bg-success/15 text-success",
        destructive: "border-transparent bg-destructive/15 text-destructive",
      },
    },
    defaultVariants: {
      variant: "default",
    },
  },
);

export type BadgeVariants = VariantProps<typeof badgeVariants>;
