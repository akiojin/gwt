import React, { useCallback, useEffect, useMemo, useState } from "react";
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
  const [showStats, setShowStats] = useState(true);
  const [statsPosition, setStatsPosition] = useState({ x: 20, y: 20 });
  const [isDragging, setIsDragging] = useState(false);
  const [dragOffset, setDragOffset] = useState({ x: 0, y: 0 });

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

  const handleStatsMouseDown = (e: React.MouseEvent<HTMLDivElement>) => {
    if ((e.target as HTMLElement).closest('.overlay-panel__close')) {
      return;
    }
    setIsDragging(true);
    const rect = e.currentTarget.getBoundingClientRect();
    setDragOffset({
      x: e.clientX - rect.left,
      y: e.clientY - rect.top,
    });
  };

  useEffect(() => {
    const handleMouseMove = (e: MouseEvent) => {
      if (!isDragging) return;
      setStatsPosition({
        x: e.clientX - dragOffset.x,
        y: e.clientY - dragOffset.y,
      });
    };

    const handleMouseUp = () => {
      setIsDragging(false);
    };

    if (isDragging) {
      document.addEventListener('mousemove', handleMouseMove);
      document.addEventListener('mouseup', handleMouseUp);
    }

    return () => {
      document.removeEventListener('mousemove', handleMouseMove);
      document.removeEventListener('mouseup', handleMouseUp);
    };
  }, [isDragging, dragOffset]);

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
    <div className="app-shell app-shell--fullscreen">
      <main className="page-content page-content--fullscreen">
        {/* Stats overlay */}
        {showStats && (
          <div
            className="overlay-panel overlay-panel--stats"
            style={{
              left: `${statsPosition.x}px`,
              top: `${statsPosition.y}px`,
              right: 'auto',
              userSelect: isDragging ? 'none' : 'auto',
            }}
            onMouseDown={handleStatsMouseDown}
          >
            <div className="overlay-panel__header">
              <h2>Statistics</h2>
              <button
                type="button"
                className="overlay-panel__close"
                onClick={() => setShowStats(false)}
                aria-label="Close statistics"
              >
                ×
              </button>
            </div>
            <div className="overlay-panel__content">
              <div className="stat-item">
                <span className="stat-item__label">Total branches</span>
                <span className="stat-item__value">{numberFormatter.format(metrics.total)}</span>
              </div>
              <div className="stat-item">
                <span className="stat-item__label">Worktrees ready</span>
                <span className="stat-item__value">{numberFormatter.format(metrics.worktrees)}</span>
              </div>
              <div className="stat-item">
                <span className="stat-item__label">Remote tracking</span>
                <span className="stat-item__value">{numberFormatter.format(metrics.remote)}</span>
              </div>
              <div className="stat-item">
                <span className="stat-item__label">Up-to-date</span>
                <span className="stat-item__value">{numberFormatter.format(metrics.healthy)}</span>
              </div>
            </div>
          </div>
        )}

        {/* Control panel */}
        <div className="overlay-panel overlay-panel--controls">
          <div className="control-group">
            <label className="search-field">
              <span className="search-field__icon">🔍</span>
              <input
                type="search"
                className="search-field__input"
                placeholder="Search branches..."
                value={query}
                onChange={(event) => setQuery(event.target.value)}
              />
            </label>
            <span className="control-badge">
              {numberFormatter.format(filteredBranches.length)} / {numberFormatter.format(metrics.total)}
            </span>
          </div>

          <div className="control-group">
            <div className="view-mode-toggle">
              <button
                type="button"
                className={`view-mode-toggle__btn ${viewMode === "graph" ? "is-active" : ""}`}
                onClick={() => setViewMode("graph")}
              >
                🌐 Graph
              </button>
              <button
                type="button"
                className={`view-mode-toggle__btn ${viewMode === "list" ? "is-active" : ""}`}
                onClick={() => setViewMode("list")}
              >
                📋 List
              </button>
            </div>
          </div>

          {(baseFilter || divergenceFilter) && (
            <div className="control-group">
              <div className="filter-chips">
                {baseFilter && (
                  <button
                    type="button"
                    className="filter-chip"
                    onClick={() => setBaseFilter(null)}
                  >
                    base: {baseFilter} <span>×</span>
                  </button>
                )}
                {divergenceFilter && (
                  <button
                    type="button"
                    className="filter-chip"
                    onClick={() => setDivergenceFilter(null)}
                  >
                    {divergenceFilter} <span>×</span>
                  </button>
                )}
              </div>
            </div>
          )}

          <div className="control-group">
            <Link to="/config" className="control-link">
              ⚙️ Settings
            </Link>
            {!showStats && (
              <button
                type="button"
                className="control-link"
                onClick={() => setShowStats(true)}
              >
                📊 Stats
              </button>
            )}
          </div>
        </div>

        {/* List view overlay */}
        {viewMode === "list" && (
          <div className="overlay-panel overlay-panel--list">
            <div className="overlay-panel__header">
              <h2>Branches</h2>
            </div>
            <div className="overlay-panel__content overlay-panel__content--scrollable">
              {pageState ? (
                <div className="empty-message">
                  <h3>{pageState.title}</h3>
                  <p>{pageState.description}</p>
                </div>
              ) : filteredBranches.length === 0 ? (
                <div className="empty-message">
                  <h3>No branches found</h3>
                  <p>Adjust your search or filters</p>
                </div>
              ) : (
                <div className="branch-list">
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
              )}
            </div>
          </div>
        )}

        {/* Graph - always rendered as background */}
        {!pageState && branches.length > 0 && (
          <div className="graph-container">
            <BranchGraph
              branches={filteredBranches.length ? filteredBranches : branches}
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
