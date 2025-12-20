import React from "react";
import { cn } from "@/lib/utils";

interface PageHeaderProps {
  eyebrow?: string;
  title: string;
  subtitle?: string;
  actions?: React.ReactNode;
  children?: React.ReactNode;
  className?: string;
}

export function PageHeader({
  eyebrow,
  title,
  subtitle,
  actions,
  children,
  className,
}: PageHeaderProps) {
  return (
    <header
      className={cn("border-b border-border bg-card/50 px-6 py-8", className)}
    >
      <div className="mx-auto max-w-7xl">
        {eyebrow && (
          <p className="mb-2 text-xs font-medium uppercase tracking-widest text-muted-foreground">
            {eyebrow}
          </p>
        )}
        <div className="flex flex-wrap items-start justify-between gap-4">
          <div>
            <h1 className="text-2xl font-bold tracking-tight">{title}</h1>
            {subtitle && (
              <p className="mt-1 text-sm text-muted-foreground">{subtitle}</p>
            )}
            {children}
          </div>
          {actions && <div className="flex gap-2">{actions}</div>}
        </div>
      </div>
    </header>
  );
}
