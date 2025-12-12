import React, { useMemo, useState } from "react";
import { Link } from "react-router-dom";
import { useBranches } from "../hooks/useBranches";
import { BranchGraph } from "../components/BranchGraph";
import { PageHeader } from "@/components/common/PageHeader";
import { MetricCard } from "@/components/common/MetricCard";
import { SearchInput } from "@/components/common/SearchInput";
import { Card, CardHeader, CardContent, CardFooter } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { cn } from "@/lib/utils";
import type { Branch } from "../../../../types/api.js";

const numberFormatter = new Intl.NumberFormat("ja-JP");

export function BranchListPage() {
  const { data, isLoading, error } = useBranches();
  const [query, setQuery] = useState("");

  const branches = data ?? [];

  const metrics = useMemo(() => {
    const worktrees = branches.filter((b) => Boolean(b.worktreePath)).length;
    const remote = branches.filter((b) => b.type === "remote").length;
    const healthy = branches.filter((b) => b.divergence?.upToDate).length;

    return { total: branches.length, worktrees, remote, healthy };
  }, [branches]);

  const normalizedQuery = query.trim().toLowerCase();

  const filteredBranches = useMemo(() => {
    if (!normalizedQuery) return branches;

    return branches.filter((branch) => {
      const haystack = [
        branch.name,
        branch.type,
        branch.mergeStatus,
        branch.commitMessage ?? "",
        branch.worktreePath ?? "",
      ]
        .join(" ")
        .toLowerCase();
      return haystack.includes(normalizedQuery);
    });
  }, [branches, normalizedQuery]);

  // Loading state
  if (isLoading) {
    return (
      <div className="min-h-screen bg-background">
        <PageHeader
          eyebrow="WORKTREE DASHBOARD"
          title="gwt Control Center"
          subtitle="Loading branch data..."
        />
        <main className="mx-auto max-w-7xl px-6 py-8">
          <div className="flex items-center justify-center py-20">
            <div className="text-center">
              <div className="mb-4 text-4xl">⏳</div>
              <p className="text-muted-foreground">Loading branches...</p>
            </div>
          </div>
        </main>
      </div>
    );
  }

  // Error state
  if (error) {
    return (
      <div className="min-h-screen bg-background">
        <PageHeader
          eyebrow="WORKTREE DASHBOARD"
          title="gwt Control Center"
        />
        <main className="mx-auto max-w-7xl px-6 py-8">
          <Alert variant="destructive">
            <AlertDescription>
              {error instanceof Error ? error.message : "Failed to load branches"}
            </AlertDescription>
          </Alert>
        </main>
      </div>
    );
  }

  // Empty state
  if (!branches.length) {
    return (
      <div className="min-h-screen bg-background">
        <PageHeader
          eyebrow="WORKTREE DASHBOARD"
          title="gwt Control Center"
        />
        <main className="mx-auto max-w-7xl px-6 py-8">
          <Card className="border-dashed">
            <CardContent className="flex flex-col items-center justify-center py-12">
              <div className="mb-4 text-4xl">📭</div>
              <h3 className="mb-2 text-lg font-semibold">No branches found</h3>
              <p className="text-sm text-muted-foreground">
                Try running <code className="rounded bg-muted px-1">git fetch origin</code> to sync branches.
              </p>
            </CardContent>
          </Card>
        </main>
      </div>
    );
  }

  return (
    <div className="min-h-screen bg-background">
      <PageHeader
        eyebrow="WORKTREE DASHBOARD"
        title="gwt Control Center"
        subtitle="Manage Git branches and AI tools from your browser"
      />

      <main className="mx-auto max-w-7xl space-y-6 px-6 py-8">
        {/* Branch Graph */}
        {filteredBranches.length > 0 && (
          <BranchGraph branches={filteredBranches} />
        )}

        {/* Metrics Grid */}
        <section className="grid grid-cols-2 gap-4 md:grid-cols-4">
          <MetricCard
            label="Total Branches"
            value={numberFormatter.format(metrics.total)}
            hint="Local + Remote"
          />
          <MetricCard
            label="Active Worktrees"
            value={numberFormatter.format(metrics.worktrees)}
            hint="Ready for AI tools"
          />
          <MetricCard
            label="Remote Tracking"
            value={numberFormatter.format(metrics.remote)}
            hint="Synced with origin"
          />
          <MetricCard
            label="Up to Date"
            value={numberFormatter.format(metrics.healthy)}
            hint="No divergence"
          />
        </section>

        {/* Search */}
        <SearchInput
          value={query}
          onChange={setQuery}
          placeholder="Search branches by name, type, or commit..."
          count={{ filtered: filteredBranches.length, total: metrics.total }}
        />

        {/* Branch Grid */}
        {filteredBranches.length === 0 ? (
          <Card className="border-dashed">
            <CardContent className="flex flex-col items-center justify-center py-12">
              <h3 className="mb-2 text-lg font-semibold">No matching branches</h3>
              <p className="text-sm text-muted-foreground">
                Try a different search term or clear the filter.
              </p>
            </CardContent>
          </Card>
        ) : (
          <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
            {filteredBranches.map((branch) => (
              <BranchCardItem key={branch.name} branch={branch} />
            ))}
          </div>
        )}
      </main>
    </div>
  );
}

// Extracted Branch Card component
function BranchCardItem({ branch }: { branch: Branch }) {
  return (
    <Card className="flex flex-col transition-colors hover:border-muted-foreground/50">
      <CardHeader className="pb-3">
        <div className="flex items-start justify-between gap-2">
          <div className="min-w-0 flex-1">
            <p className="text-xs font-medium uppercase tracking-wider text-muted-foreground">
              {branch.type === "local" ? "Local" : "Remote"} Branch
            </p>
            <h3 className="mt-1 truncate font-semibold" title={branch.name}>
              {branch.name}
            </h3>
          </div>
          <div className="flex flex-wrap justify-end gap-1">
            <Badge variant={branch.type === "local" ? "local" : "remote"}>
              {branch.type === "local" ? "L" : "R"}
            </Badge>
            {branch.worktreePath && (
              <Badge variant="success">WT</Badge>
            )}
          </div>
        </div>
      </CardHeader>

      <CardContent className="flex-1 pb-3">
        <p className="line-clamp-2 text-sm text-muted-foreground">
          {branch.commitMessage ?? "No commit message"}
        </p>

        {branch.divergence && (
          <div className="mt-3 flex flex-wrap gap-1.5">
            {branch.divergence.ahead > 0 && (
              <Badge variant="outline" className="text-xs">
                ↑ {branch.divergence.ahead}
              </Badge>
            )}
            {branch.divergence.behind > 0 && (
              <Badge variant="outline" className="text-xs">
                ↓ {branch.divergence.behind}
              </Badge>
            )}
            <Badge
              variant={branch.divergence.upToDate ? "success" : "warning"}
              className="text-xs"
            >
              {branch.divergence.upToDate ? "Up to date" : "Needs sync"}
            </Badge>
          </div>
        )}
      </CardContent>

      <CardFooter className="pt-0">
        <Button variant="ghost" size="sm" asChild className="w-full">
          <Link to={`/${encodeURIComponent(branch.name)}`}>
            View Details →
          </Link>
        </Button>
      </CardFooter>
    </Card>
  );
}
