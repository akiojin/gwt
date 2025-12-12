import React, { useMemo } from "react";
import { Link } from "react-router-dom";
import { Card, CardHeader, CardContent } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { cn } from "@/lib/utils";
import type { Branch } from "../../../../types/api.js";

const UNKNOWN_BASE = "__unknown__";

interface Lane {
  id: string;
  baseLabel: string;
  baseNode: Branch | null;
  nodes: Branch[];
  isSyntheticBase: boolean;
}

interface BranchGraphProps {
  branches: Branch[];
}

function formatBranchLabel(branch: Branch): string {
  return branch.name.length > 32
    ? `${branch.name.slice(0, 29)}...`
    : branch.name;
}

function getDivergenceLabel(branch: Branch): string {
  if (!branch.divergence) {
    return "divergence: n/a";
  }
  const { ahead, behind, upToDate } = branch.divergence;
  if (upToDate) {
    return "divergence: up-to-date";
  }
  return `divergence: +${ahead} / -${behind}`;
}

export function BranchGraph({ branches }: BranchGraphProps) {
  const branchMap = useMemo(() => {
    return new Map(branches.map((branch) => [branch.name, branch]));
  }, [branches]);

  const referencedBases = useMemo(() => {
    const baseSet = new Set<string>();
    branches.forEach((branch) => {
      if (branch.baseBranch) {
        baseSet.add(branch.baseBranch);
      }
    });
    return baseSet;
  }, [branches]);

  const lanes = useMemo<Lane[]>(() => {
    const laneMap = new Map<string, Lane>();

    branches.forEach((branch) => {
      const base = branch.baseBranch ?? UNKNOWN_BASE;

      if (!branch.baseBranch && referencedBases.has(branch.name)) {
        return;
      }

      if (!laneMap.has(base)) {
        const baseNode =
          base !== UNKNOWN_BASE ? (branchMap.get(base) ?? null) : null;
        laneMap.set(base, {
          id: base,
          baseLabel: base === UNKNOWN_BASE ? "ベース不明" : base,
          baseNode,
          nodes: [],
          isSyntheticBase: baseNode === null,
        });
      }

      laneMap.get(base)?.nodes.push(branch);
    });

    return Array.from(laneMap.values()).sort((a, b) => {
      if (a.id === UNKNOWN_BASE) return 1;
      if (b.id === UNKNOWN_BASE) return -1;
      return a.baseLabel.localeCompare(b.baseLabel, "ja");
    });
  }, [branches, branchMap, referencedBases]);

  if (!lanes.length) {
    return (
      <Card className="border-dashed">
        <CardContent className="flex flex-col items-center justify-center py-12 text-center">
          <p className="text-muted-foreground">グラフ表示できるブランチがありません。</p>
          <p className="text-sm text-muted-foreground">
            fetch済みのブランチやWorktreeを追加すると関係図が表示されます。
          </p>
        </CardContent>
      </Card>
    );
  }

  return (
    <Card>
      <CardHeader className="pb-4">
        <div className="flex flex-wrap items-start justify-between gap-4">
          <div>
            <p className="text-xs font-medium uppercase tracking-wider text-muted-foreground">
              BRANCH GRAPH
            </p>
            <h2 className="mt-1 text-lg font-semibold">
              ベースブランチの関係をグラフィカルに把握
            </h2>
            <p className="mt-1 text-sm text-muted-foreground">
              baseRef、Git upstream、merge-baseヒューリスティクスを用いて推定したベースブランチ単位で派生ノードをレーン表示します。
            </p>
          </div>
          <div className="flex flex-wrap gap-2">
            <Badge variant="outline">Base</Badge>
            <Badge variant="local">Local</Badge>
            <Badge variant="remote">Remote</Badge>
            <Badge variant="success">Worktree</Badge>
          </div>
        </div>
      </CardHeader>

      <CardContent className="space-y-4">
        {lanes.map((lane) => (
          <article key={lane.id} className="rounded-lg border bg-muted/30 p-4">
            <div className="mb-3 flex items-center justify-between">
              <div className="flex items-center gap-2">
                <span className="font-semibold">{lane.baseLabel}</span>
                {lane.baseNode && (
                  <Badge variant={lane.baseNode.type === "local" ? "local" : "remote"} className="text-xs">
                    {lane.baseNode.type === "local" ? "LOCAL" : "REMOTE"}
                  </Badge>
                )}
                {lane.isSyntheticBase && (
                  <Badge variant="outline" className="text-xs text-muted-foreground">
                    推定のみ
                  </Badge>
                )}
              </div>
              <span className="text-sm text-muted-foreground">
                {lane.nodes.length} branch{lane.nodes.length > 1 ? "es" : ""}
              </span>
            </div>

            <div className="flex flex-wrap gap-2">
              {renderBaseNode(lane)}
              {lane.nodes.map((branch) => (
                <BranchNode key={branch.name} branch={branch} />
              ))}
            </div>
          </article>
        ))}
      </CardContent>
    </Card>
  );
}

function renderBaseNode(lane: Lane) {
  const label = lane.baseLabel === "ベース不明" ? "Unknown base" : lane.baseLabel;

  const content = (
    <div
      className={cn(
        "group relative rounded-md border bg-card px-3 py-2 transition-colors hover:border-muted-foreground/50",
        lane.baseNode?.type === "local" && "border-l-2 border-l-local",
        lane.baseNode?.type === "remote" && "border-l-2 border-l-remote"
      )}
    >
      <span className="block truncate text-sm font-medium">{label}</span>
      <span className="text-xs text-muted-foreground">BASE</span>

      {/* Tooltip */}
      <div className="invisible absolute bottom-full left-0 z-10 mb-2 w-48 rounded-md border bg-popover p-2 text-xs shadow-md group-hover:visible">
        <p className="font-medium">{label}</p>
        <p className="text-muted-foreground">
          {lane.baseNode ? `type: ${lane.baseNode.type}` : "推定されたベースブランチ"}
        </p>
      </div>
    </div>
  );

  if (lane.baseNode) {
    return (
      <Link
        key={`base-${lane.id}`}
        to={`/${encodeURIComponent(lane.baseNode.name)}`}
        className="block"
        aria-label={`ベースブランチ ${lane.baseNode.name} を開く`}
      >
        {content}
      </Link>
    );
  }

  return <div key={`base-${lane.id}`}>{content}</div>;
}

function BranchNode({ branch }: { branch: Branch }) {
  const node = (
    <div
      className={cn(
        "group relative rounded-md border bg-card px-3 py-2 transition-colors hover:border-muted-foreground/50",
        branch.type === "local" && "border-l-2 border-l-local",
        branch.type === "remote" && "border-l-2 border-l-remote",
        branch.mergeStatus === "merged" && "opacity-60"
      )}
    >
      <span className="block truncate text-sm font-medium">
        {formatBranchLabel(branch)}
      </span>
      <span className="text-xs text-muted-foreground">
        {branch.worktreePath ? "Worktree" : "No Worktree"}
      </span>

      {/* Tooltip */}
      <div className="invisible absolute bottom-full left-0 z-10 mb-2 w-56 rounded-md border bg-popover p-2 text-xs shadow-md group-hover:visible">
        <p className="font-medium">{branch.name}</p>
        <p className="text-muted-foreground">base: {branch.baseBranch ?? "unknown"}</p>
        <p className="text-muted-foreground">{getDivergenceLabel(branch)}</p>
        <p className="text-muted-foreground">{branch.worktreePath ?? "Worktree未作成"}</p>
      </div>
    </div>
  );

  return (
    <Link
      to={`/${encodeURIComponent(branch.name)}`}
      className="block"
      aria-label={`${branch.name} の詳細を開く`}
    >
      {node}
    </Link>
  );
}
