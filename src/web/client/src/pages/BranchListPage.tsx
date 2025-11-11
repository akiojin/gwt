import React, { useCallback, useMemo, useState } from "react";
import { Link } from "react-router-dom";
import { useBranches } from "../hooks/useBranches";
import { BranchGraph } from "../components/BranchGraph";
import { AIToolLaunchModal } from "../components/AIToolLaunchModal";
import type { Branch } from "../../../../types/api.js";

const numberFormatter = new Intl.NumberFormat("en-US");

const BRANCH_TYPE_LABEL: Record<Branch["type"], string> = {
  local: "Local",
  remote: "Remote",
};

const MERGE_STATUS_LABEL: Record<Branch["mergeStatus"], string> = {
  merged: "Merged",
  unmerged: "Not merged",
  unknown: "Unknown",
};

const MERGE_STATUS_TONE: Record<Branch["mergeStatus"], "success" | "warning" | "muted"> = {
  merged: "success",
  unmerged: "warning",
  unknown: "muted",
};

interface PageStateMessage {
  title: string;
  description: string;
}

const SEARCH_PLACEHOLDER = "Search by branch name or type...";

type ViewMode = "graph" | "list";
type DivergenceFilter = "ahead" | "behind" | "upToDate";

function canonicalName(name?: string | null): string | null {
  if (!name) {
    return null;
  }
  return name.replace(/^origin\//, "");
}

export function BranchListPage() {
  const { data, isLoading, error } = useBranches();
  const [query, setQuery] = useState("");
  const [selectedBranch, setSelectedBranch] = useState<Branch | null>(null);
  const [viewMode, setViewMode] = useState<ViewMode>("graph");
  const [baseFilter, setBaseFilter] = useState<string | null>(null);
  const [divergenceFilter, setDivergenceFilter] = useState<DivergenceFilter | null>(null);

  const handleBranchSelection = useCallback((branch: Branch) => {
    setSelectedBranch(branch);
  }, []);

  const handleCardKeyDown = useCallback(
    (event: React.KeyboardEvent<HTMLElement>, branch: Branch) => {
      if (event.key === "Enter" || event.key === " ") {
        event.preventDefault();
        handleBranchSelection(branch);
      }
    },
    [handleBranchSelection],
  );

  const branches = data ?? [];

  const metrics = useMemo(() => {
    const worktrees = branches.filter((branch) => Boolean(branch.worktreePath)).length;
    const remote = branches.filter((branch) => branch.type === "remote").length;
    const healthy = branches.filter((branch) => branch.divergence?.upToDate).length;

    return {
      total: branches.length,
      worktrees,
      remote,
      healthy,
    };
  }, [branches]);

  const normalizedQuery = query.trim().toLowerCase();

  const matchesDivergence = (branch: Branch) => {
    if (!divergenceFilter) {
      return true;
    }
    if (!branch.divergence) {
      return false;
    }
    switch (divergenceFilter) {
      case "upToDate":
        return Boolean(branch.divergence.upToDate);
      case "ahead":
        return (branch.divergence.ahead ?? 0) > 0;
      case "behind":
        return (branch.divergence.behind ?? 0) > 0;
      default:
        return true;
    }
  };

  const filteredBranches = useMemo(() => {
    const baseQueryFiltered = branches.filter((branch) => {
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

    const baseMatched = baseFilter
      ? baseQueryFiltered.filter((branch) => {
          const canonicalBranchName = canonicalName(branch.name);
          if (canonicalBranchName === baseFilter) {
            return true;
          }
          if (baseFilter === "detached") {
            return !canonicalName(branch.baseBranch);
          }
          return canonicalName(branch.baseBranch) === baseFilter;
        })
      : baseQueryFiltered;

    return baseMatched.filter(matchesDivergence);
  }, [branches, normalizedQuery, baseFilter, divergenceFilter]);

  const pageState: PageStateMessage | null = useMemo(() => {
    if (isLoading) {
      return {
        title: "Loading",
        description: "Fetching latest branch data...",
      };
    }

    if (error) {
      return {
        title: "Failed to load branches",
        description:
          error instanceof Error ? error.message : "Unknown error occurred.",
      };
    }

    if (!branches.length) {
      return {
        title: "No branches found",
        description: "Run git fetch origin to pull the latest branches.",
      };
    }

    return null;
  }, [branches.length, error, isLoading]);

  return (
    <div className="app-shell">
      <header className="page-hero">
        <p className="page-hero__eyebrow">WORKTREE DASHBOARD</p>
        <h1>Claude Worktree Control Center</h1>
        <p>
          Manage local git branches and AI tools from the browser while keeping worktree status visible at a glance.
        </p>
        <div className="page-hero__meta">Real-time status overview</div>
        <div className="page-hero__actions">
          <Link to="/config" className="button button--secondary">
            Custom Tool Settings
          </Link>
        </div>
      </header>

      <main className="page-content">
        <section className="metrics-grid">
          <article className="metric-card">
            <p className="metric-card__label">Total branches</p>
            <p className="metric-card__value" data-testid="metric-total">
              {numberFormatter.format(metrics.total)}
            </p>
            <p className="metric-card__hint">Local + Remote</p>
          </article>
          <article className="metric-card">
            <p className="metric-card__label">Worktrees ready</p>
            <p className="metric-card__value" data-testid="metric-worktrees">
              {numberFormatter.format(metrics.worktrees)}
            </p>
            <p className="metric-card__hint">Launch-ready worktrees</p>
          </article>
          <article className="metric-card">
            <p className="metric-card__label">Remote tracking</p>
            <p className="metric-card__value">
              {numberFormatter.format(metrics.remote)}
            </p>
            <p className="metric-card__hint">Sync status vs origin</p>
          </article>
          <article className="metric-card">
            <p className="metric-card__label">Up-to-date commits</p>
            <p className="metric-card__value">
              {numberFormatter.format(metrics.healthy)}
            </p>
            <p className="metric-card__hint">Branches with divergence 0</p>
          </article>
        </section>

        <section className="toolbar">
          <label className="toolbar__field">
            <span className="toolbar__icon" aria-hidden="true">
              🔍
            </span>
            <input
              type="search"
              className="search-input"
              placeholder={SEARCH_PLACEHOLDER}
              value={query}
              onChange={(event) => setQuery(event.target.value)}
            />
          </label>
          <span className="toolbar__count">
            {numberFormatter.format(filteredBranches.length)} / {" "}
            {numberFormatter.format(metrics.total)} branches
          </span>
          <div className="view-toggle" role="group" aria-label="Toggle view mode">
            <button
              type="button"
              className={`view-toggle__button ${viewMode === "graph" ? "is-active" : ""}`}
              onClick={() => setViewMode("graph")}
              aria-pressed={viewMode === "graph"}
            >
              Graph view
            </button>
            <button
              type="button"
              className={`view-toggle__button ${viewMode === "list" ? "is-active" : ""}`}
              onClick={() => setViewMode("list")}
              aria-pressed={viewMode === "list"}
            >
              List view
            </button>
          </div>
          <div className="filter-pill-group">
            {baseFilter && (
              <button
                type="button"
                className="filter-pill"
                onClick={() => setBaseFilter(null)}
                aria-label={`Clear base filter ${baseFilter}`}
              >
                base: {baseFilter}
                <span aria-hidden="true">×</span>
              </button>
            )}
            {divergenceFilter && (
              <button
                type="button"
                className="filter-pill"
                onClick={() => setDivergenceFilter(null)}
                aria-label={`Clear divergence filter ${divergenceFilter}`}
              >
                divergence: {divergenceFilter}
                <span aria-hidden="true">×</span>
              </button>
            )}
          </div>
        </section>

        {pageState ? (
          <div className="page-state page-state--card">
            <h2>{pageState.title}</h2>
            <p>{pageState.description}</p>
          </div>
        ) : filteredBranches.length === 0 ? (
          <div className="empty-state">
            <h3>No branches match your filters</h3>
            <p>
              Adjust the search query or filters (type, divergence, base) and try again.
            </p>
          </div>
        ) : viewMode === "list" ? (
          <div className="branch-grid">
            {filteredBranches.map((branch) => (
              <article
                key={branch.name}
                className="branch-card branch-card--interactive"
                role="button"
                tabIndex={0}
                aria-label={`Configure AI tool for ${branch.name}`}
                onClick={() => handleBranchSelection(branch)}
                onKeyDown={(event) => handleCardKeyDown(event, branch)}
              >
                <div className="branch-card__header">
                  <div>
                    <p className="branch-card__eyebrow">
                      {BRANCH_TYPE_LABEL[branch.type]} branch
                    </p>
                    <h2>{branch.name}</h2>
                  </div>
                  <div className="badge-group">
                    <span className={`status-badge status-badge--${branch.type}`}>
                      {BRANCH_TYPE_LABEL[branch.type]}
                    </span>
                    <span className={`status-badge status-badge--${MERGE_STATUS_TONE[branch.mergeStatus]}`}>
                      {MERGE_STATUS_LABEL[branch.mergeStatus]}
                    </span>
                    <span
                      className={`status-badge ${
                        branch.worktreePath
                          ? "status-badge--success"
                          : "status-badge--muted"
                      }`}
                    >
                      {branch.worktreePath ? "Worktree ready" : "No worktree"}
                    </span>
                  </div>
                </div>

                <p className="branch-card__commit">
                  {branch.commitMessage ?? "No commit message"}
                </p>

                <dl className="metadata-grid metadata-grid--compact">
                  <div>
                    <dt>Latest commit</dt>
                    <dd>{branch.commitHash.slice(0, 7)}</dd>
                  </div>
                  <div>
                    <dt>Author</dt>
                    <dd>{branch.author ?? "N/A"}</dd>
                  </div>
                  <div>
                    <dt>Worktree</dt>
                    <dd>{branch.worktreePath ?? "Not created"}</dd>
                  </div>
                </dl>

                {branch.divergence && (
                  <div className="pill-group">
                    <span className="pill">Ahead {branch.divergence.ahead}</span>
                    <span className="pill">Behind {branch.divergence.behind}</span>
                    <span
                      className={`pill ${
                        branch.divergence.upToDate ? "pill--success" : "pill--warning"
                      }`}
                    >
                      {branch.divergence.upToDate ? "Up-to-date" : "Needs sync"}
                    </span>
                  </div>
                )}

                <div className="branch-card__actions">
                  <button
                    type="button"
                    className="button button--primary"
                    onClick={(event) => {
                      event.stopPropagation();
                      handleBranchSelection(branch);
                    }}
                  >
                    Launch AI tool
                  </button>
                  <Link
                    className="button button--ghost"
                    to={`/${encodeURIComponent(branch.name)}`}
                    onClick={(event) => event.stopPropagation()}
                  >
                    View session
                  </Link>
                  <span
                    className={`info-pill ${
                      branch.worktreePath ? "info-pill--success" : "info-pill--warning"
                    }`}
                  >
                    {branch.worktreePath ?? "Worktree missing"}
                  </span>
                </div>
              </article>
            ))}
          </div>
        ) : (
          <div className="page-state page-state--card">
            <p>Graph view is active.</p>
          </div>
        )}

        {!pageState && branches.length > 0 && viewMode === "graph" && (
          <div className="graph-container">
            <BranchGraph
              branches={filteredBranches.length ? filteredBranches : branches}
              activeBase={baseFilter}
              onBaseFilterChange={setBaseFilter}
              activeDivergence={divergenceFilter}
              onDivergenceFilterChange={setDivergenceFilter}
              onSelectBranch={handleBranchSelection}
            />
          </div>
        )}
      </main>
      {selectedBranch && (
        <AIToolLaunchModal branch={selectedBranch} onClose={() => setSelectedBranch(null)} />
      )}
    </div>
  );
}
