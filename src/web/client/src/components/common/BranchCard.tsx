import { Link } from "react-router-dom";
import { Card, CardHeader, CardContent, CardFooter } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { cn } from "@/lib/utils";

interface BranchCardProps {
  name: string;
  type: "local" | "remote" | "both";
  isProtected?: boolean;
  hasWorktree?: boolean;
  lastCommit?: string;
  lastCommitDate?: string;
  ahead?: number;
  behind?: number;
  merged?: boolean;
  href?: string;
  className?: string;
}

export function BranchCard({
  name,
  type,
  isProtected,
  hasWorktree,
  lastCommit,
  lastCommitDate,
  ahead,
  behind,
  merged,
  href,
  className,
}: BranchCardProps) {
  const content = (
    <Card
      className={cn(
        "transition-colors hover:border-muted-foreground/30",
        className
      )}
    >
      <CardHeader className="pb-2">
        <div className="flex items-start justify-between gap-2">
          <div className="min-w-0 flex-1">
            <p className="text-xs font-medium uppercase tracking-wider text-muted-foreground">
              Branch
            </p>
            <h3 className="mt-1 truncate text-sm font-semibold">{name}</h3>
          </div>
          <div className="flex flex-wrap gap-1">
            {type === "local" && <Badge variant="local">Local</Badge>}
            {type === "remote" && <Badge variant="remote">Remote</Badge>}
            {type === "both" && (
              <>
                <Badge variant="local">L</Badge>
                <Badge variant="remote">R</Badge>
              </>
            )}
          </div>
        </div>
      </CardHeader>
      <CardContent className="py-2">
        {lastCommit && (
          <p className="line-clamp-2 text-xs text-muted-foreground">
            {lastCommit}
          </p>
        )}
      </CardContent>
      <CardFooter className="flex flex-wrap gap-2 pt-2">
        {isProtected && (
          <Badge variant="warning" className="text-xs">
            Protected
          </Badge>
        )}
        {hasWorktree && (
          <Badge variant="success" className="text-xs">
            Worktree
          </Badge>
        )}
        {merged && (
          <Badge variant="secondary" className="text-xs">
            Merged
          </Badge>
        )}
        {(ahead !== undefined && ahead > 0) && (
          <Badge variant="outline" className="text-xs">
            ↑{ahead}
          </Badge>
        )}
        {(behind !== undefined && behind > 0) && (
          <Badge variant="outline" className="text-xs">
            ↓{behind}
          </Badge>
        )}
        {lastCommitDate && (
          <span className="ml-auto text-xs text-muted-foreground">
            {lastCommitDate}
          </span>
        )}
      </CardFooter>
    </Card>
  );

  if (href) {
    return (
      <Link to={href} className="block">
        {content}
      </Link>
    );
  }

  return content;
}
