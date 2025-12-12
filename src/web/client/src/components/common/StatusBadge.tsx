import React from "react";
import { Badge } from "@/components/ui/badge";
import { cn } from "@/lib/utils";

type StatusType = "local" | "remote" | "success" | "warning" | "muted" | "default";

interface StatusBadgeProps {
  status: StatusType;
  children: React.ReactNode;
  className?: string;
}

export function StatusBadge({ status, children, className }: StatusBadgeProps) {
  const variantMap: Record<StatusType, "local" | "remote" | "success" | "warning" | "secondary" | "default"> = {
    local: "local",
    remote: "remote",
    success: "success",
    warning: "warning",
    muted: "secondary",
    default: "default",
  };

  return (
    <Badge variant={variantMap[status]} className={cn(className)}>
      {children}
    </Badge>
  );
}
