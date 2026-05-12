import * as React from 'react'
import { cva, type VariantProps } from 'class-variance-authority'

import { cn } from '@/lib/utils'

const badgeVariants = cva(
  'inline-flex items-center gap-1 rounded-full border px-2.5 py-0.5 text-xs font-medium transition-colors focus:outline-none focus:ring-2 focus:ring-ring focus:ring-offset-2 [&_svg]:size-3 [&_svg]:-ms-0.5',
  {
    variants: {
      variant: {
        default: 'border-transparent bg-primary/10 text-primary hover:bg-primary/15',
        solid: 'border-transparent bg-primary text-primary-foreground hover:bg-primary/90',
        secondary: 'border-transparent bg-secondary text-secondary-foreground hover:bg-secondary/80',
        destructive: 'border-transparent bg-destructive/12 text-destructive',
        success:
          'border-transparent bg-success/12 text-success dark:bg-success/18 dark:text-[hsl(var(--success))]',
        warning:
          'border-transparent bg-warning/15 text-warning dark:bg-warning/20 dark:text-[hsl(var(--warning))]',
        outline: 'text-foreground bg-transparent',
      },
    },
    defaultVariants: {
      variant: 'default',
    },
  },
)

export interface BadgeProps
  extends React.HTMLAttributes<HTMLDivElement>,
    VariantProps<typeof badgeVariants> {}

function Badge({ className, variant, ...props }: BadgeProps) {
  return <div className={cn(badgeVariants({ variant }), className)} {...props} />
}

export { Badge, badgeVariants }
