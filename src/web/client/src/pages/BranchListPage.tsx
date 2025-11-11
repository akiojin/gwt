import React, { useCallback, useMemo, useState } from "react";
import { Link } from "react-router-dom";
import { useBranches } from "../hooks/useBranches";
import { BranchGraph } from "../components/BranchGraph";
import { AIToolLaunchModal } from "../components/AIToolLaunchModal";
import type { Branch } from "../../../../types/api.js";

const numberFormatter = new Intl.NumberFormat("ja-JP");

const BRANCH_TYPE_LABEL: Record<Branch["type"], string> = {
  local: "ローカル",
  remote: "リモート",
};

const MERGE_STATUS_LABEL: Record<Branch["mergeStatus"], string> = {
  merged: "マージ済み",
  unmerged: "未マージ",
  unknown: "状態不明",
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

const SEARCH_PLACEHOLDER = "ブランチ名やタイプで検索...";

type ViewMode = "graph" | "list";
type DivergenceFilter = "ahead" | "behind" | "upToDate";

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
          if (branch.name === baseFilter) {
            return true;
          }
          if (baseFilter === "detached") {
            return !branch.baseBranch;
          }
          return branch.baseBranch === baseFilter;
        })
      : baseQueryFiltered;

    return baseMatched.filter(matchesDivergence);
  }, [branches, normalizedQuery, baseFilter, divergenceFilter]);

  const pageState: PageStateMessage | null = useMemo(() => {
    if (isLoading) {
      return {
        title: "データを読み込み中",
        description: "最新のブランチ一覧を取得しています...",
      };
    }

    if (error) {
      return {
        title: "ブランチの取得に失敗しました",
        description:
          error instanceof Error ? error.message : "未知のエラーが発生しました。",
      };
    }

    if (!branches.length) {
      return {
        title: "ブランチが見つかりません",
        description: "git fetch origin などで最新のブランチを取得してください。",
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
          ローカルのGitブランチとAIツールをブラウザ上で一元管理し、Worktree状態を瞬時に
          可視化します。
        </p>
        <div className="page-hero__meta">リアルタイムで更新されるステータスビュー</div>
        <div className="page-hero__actions">
          <Link to="/config" className="button button--secondary">
            カスタムツール設定
          </Link>
        </div>
      </header>

      <main className="page-content">
        {!pageState && branches.length > 0 && viewMode === "graph" && (
          <BranchGraph
            branches={filteredBranches.length ? filteredBranches : branches}
            activeBase={baseFilter}
            onBaseFilterChange={setBaseFilter}
            activeDivergence={divergenceFilter}
            onDivergenceFilterChange={setDivergenceFilter}
            onSelectBranch={handleBranchSelection}
          />
        )}

        <section className="metrics-grid">
          <article className="metric-card">
            <p className="metric-card__label">総ブランチ数</p>
            <p className="metric-card__value" data-testid="metric-total">
              {numberFormatter.format(metrics.total)}
            </p>
            <p className="metric-card__hint">ローカル + リモート</p>
          </article>
          <article className="metric-card">
            <p className="metric-card__label">作成済みWorktree</p>
            <p className="metric-card__value" data-testid="metric-worktrees">
              {numberFormatter.format(metrics.worktrees)}
            </p>
            <p className="metric-card__hint">即座にAIツールを起動可能</p>
          </article>
          <article className="metric-card">
            <p className="metric-card__label">リモート追跡ブランチ</p>
            <p className="metric-card__value">
              {numberFormatter.format(metrics.remote)}
            </p>
            <p className="metric-card__hint">origin との同期ステータス</p>
          </article>
          <article className="metric-card">
            <p className="metric-card__label">最新コミットが最新</p>
            <p className="metric-card__value">
              {numberFormatter.format(metrics.healthy)}
            </p>
            <p className="metric-card__hint">divergence 0 のブランチ</p>
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
          <div className="view-toggle" role="group" aria-label="表示モード切替">
            <button
              type="button"
              className={`view-toggle__button ${viewMode === "graph" ? "is-active" : ""}`}
              onClick={() => setViewMode("graph")}
              aria-pressed={viewMode === "graph"}
            >
              グラフビュー
            </button>
            <button
              type="button"
              className={`view-toggle__button ${viewMode === "list" ? "is-active" : ""}`}
              onClick={() => setViewMode("list")}
              aria-pressed={viewMode === "list"}
            >
              リストビュー
            </button>
          </div>
          <div className="filter-pill-group">
            {baseFilter && (
              <button
                type="button"
                className="filter-pill"
                onClick={() => setBaseFilter(null)}
                aria-label={`${baseFilter} のフィルターを解除`}
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
                aria-label={`divergence ${divergenceFilter} のフィルターを解除`}
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
            <h3>一致するブランチがありません</h3>
            <p>
              検索条件を見直すか、タグ・ブランチタイプ・コミットメッセージなど別のキーワードを
              試してください。
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
                aria-label={`${branch.name} のAIツールを設定`}
                onClick={() => handleBranchSelection(branch)}
                onKeyDown={(event) => handleCardKeyDown(event, branch)}
              >
                <div className="branch-card__header">
                  <div>
                    <p className="branch-card__eyebrow">
                      {BRANCH_TYPE_LABEL[branch.type]}ブランチ
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
                      {branch.worktreePath ? "Worktreeあり" : "Worktree未作成"}
                    </span>
                  </div>
                </div>

                <p className="branch-card__commit">
                  {branch.commitMessage ?? "コミットメッセージがありません"}
                </p>

                <dl className="metadata-grid metadata-grid--compact">
                  <div>
                    <dt>最新コミット</dt>
                    <dd>{branch.commitHash.slice(0, 7)}</dd>
                  </div>
                  <div>
                    <dt>Author</dt>
                    <dd>{branch.author ?? "N/A"}</dd>
                  </div>
                  <div>
                    <dt>Worktree</dt>
                    <dd>{branch.worktreePath ?? "未作成"}</dd>
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
                      {branch.divergence.upToDate ? "最新" : "更新あり"}
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
                    AIツールを起動
                  </button>
                  <Link
                    className="button button--ghost"
                    to={`/${encodeURIComponent(branch.name)}`}
                    onClick={(event) => event.stopPropagation()}
                  >
                    セッションを表示
                  </Link>
                  <span
                    className={`info-pill ${
                      branch.worktreePath ? "info-pill--success" : "info-pill--warning"
                    }`}
                  >
                    {branch.worktreePath ?? "Worktree未作成"}
                  </span>
                </div>
              </article>
            ))}
          </div>
        ) : null}
      </main>
      {selectedBranch && (
        <AIToolLaunchModal branch={selectedBranch} onClose={() => setSelectedBranch(null)} />
      )}
    </div>
  );
}
