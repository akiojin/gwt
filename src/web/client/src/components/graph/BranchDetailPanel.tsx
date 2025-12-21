/**
 * ブランチ詳細パネル
 *
 * ノードクリック時に右側に表示されるサイドパネル
 */

import React from "react";
import { Link } from "react-router-dom";
import { X } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardHeader,
  CardContent,
  CardFooter,
} from "@/components/ui/card";
import { cn } from "@/lib/utils";
import type { Branch } from "../../../../../types/api.js";

interface BranchDetailPanelProps {
  branch: Branch | null;
  onClose: () => void;
  className?: string;
}

export function BranchDetailPanel({
  branch,
  onClose,
  className,
}: BranchDetailPanelProps) {
  if (!branch) return null;

  const hasWorktree = Boolean(branch.worktreePath);

  return (
    <div
      className={cn(
        "absolute right-0 top-0 h-full w-80 border-l bg-card/95 backdrop-blur-sm",
        "animate-in slide-in-from-right duration-300",
        className,
      )}
    >
      <Card className="h-full rounded-none border-0">
        <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-4">
          <div className="flex items-center gap-2">
            <Badge variant={branch.type === "local" ? "local" : "remote"}>
              {branch.type === "local" ? "Local" : "Remote"}
            </Badge>
            {hasWorktree && <Badge variant="success">Worktree</Badge>}
          </div>
          <Button
            variant="ghost"
            size="icon"
            onClick={onClose}
            className="h-8 w-8"
          >
            <X className="h-4 w-4" />
          </Button>
        </CardHeader>

        <CardContent className="space-y-4">
          {/* ブランチ名 */}
          <div>
            <p className="text-xs font-medium uppercase tracking-wider text-muted-foreground">
              Branch Name
            </p>
            <p className="mt-1 break-all font-mono text-sm">{branch.name}</p>
          </div>

          {/* ベースブランチ */}
          <div>
            <p className="text-xs font-medium uppercase tracking-wider text-muted-foreground">
              Base Branch
            </p>
            <p className="mt-1 font-mono text-sm">
              {branch.baseBranch ?? "Unknown"}
            </p>
          </div>

          {/* コミットメッセージ */}
          <div>
            <p className="text-xs font-medium uppercase tracking-wider text-muted-foreground">
              Last Commit
            </p>
            <p className="mt-1 text-sm text-muted-foreground">
              {branch.commitMessage ?? "No commit message"}
            </p>
          </div>

          {/* Divergence */}
          {branch.divergence && (
            <div>
              <p className="text-xs font-medium uppercase tracking-wider text-muted-foreground">
                Divergence
              </p>
              <div className="mt-2 flex flex-wrap gap-2">
                <Badge variant="outline" className="text-xs">
                  ↑ {branch.divergence.ahead} ahead
                </Badge>
                <Badge variant="outline" className="text-xs">
                  ↓ {branch.divergence.behind} behind
                </Badge>
                <Badge
                  variant={branch.divergence.upToDate ? "success" : "warning"}
                  className="text-xs"
                >
                  {branch.divergence.upToDate ? "Up to date" : "Needs sync"}
                </Badge>
              </div>
            </div>
          )}

          {/* Worktree パス */}
          {hasWorktree && (
            <div>
              <p className="text-xs font-medium uppercase tracking-wider text-muted-foreground">
                Worktree Path
              </p>
              <p className="mt-1 break-all font-mono text-xs text-muted-foreground">
                {branch.worktreePath}
              </p>
            </div>
          )}

          {/* マージステータス */}
          {branch.mergeStatus && (
            <div>
              <p className="text-xs font-medium uppercase tracking-wider text-muted-foreground">
                Merge Status
              </p>
              <Badge
                variant={
                  branch.mergeStatus === "merged" ? "success" : "outline"
                }
                className="mt-1"
              >
                {branch.mergeStatus}
              </Badge>
            </div>
          )}
        </CardContent>

        <CardFooter className="flex flex-col gap-2">
          <Button asChild className="w-full">
            <Link to={`/${encodeURIComponent(branch.name)}`}>View Details</Link>
          </Button>
        </CardFooter>
      </Card>
    </div>
  );
}
