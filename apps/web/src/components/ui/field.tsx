import * as React from "react"
import { cn } from "@/lib/utils"

export const Surface = React.forwardRef<HTMLDivElement, React.HTMLAttributes<HTMLDivElement>>(
  ({ className, ...props }, ref) => (
    <div ref={ref} className={cn("p-4 rounded-xl border border-border bg-card shadow-sm", className)} {...props} />
  )
)
Surface.displayName = "Surface"

export const FieldGroup = React.forwardRef<HTMLDivElement, React.HTMLAttributes<HTMLDivElement>>(
  ({ className, ...props }, ref) => (
    <div ref={ref} className={cn("flex flex-col gap-4", className)} {...props} />
  )
)
FieldGroup.displayName = "FieldGroup"

export const FieldLabel = React.forwardRef<HTMLLabelElement, React.LabelHTMLAttributes<HTMLLabelElement>>(
  ({ className, ...props }, ref) => (
    <label ref={ref} className={cn("flex flex-col gap-1.5", className)} {...props} />
  )
)
FieldLabel.displayName = "FieldLabel"

export const FieldTitle = React.forwardRef<HTMLSpanElement, React.HTMLAttributes<HTMLSpanElement>>(
  ({ className, ...props }, ref) => (
    <span ref={ref} className={cn("text-sm font-medium", className)} {...props} />
  )
)
FieldTitle.displayName = "FieldTitle"

export const FieldHint = React.forwardRef<HTMLParagraphElement, React.HTMLAttributes<HTMLParagraphElement>>(
  ({ className, ...props }, ref) => (
    <p ref={ref} className={cn("text-xs text-muted-foreground", className)} {...props} />
  )
)
FieldHint.displayName = "FieldHint"
