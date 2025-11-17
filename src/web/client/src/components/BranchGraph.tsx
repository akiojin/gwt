import React, { useMemo } from "react";
import { Link } from "react-router-dom";
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
        // ベースとして参照されている場合は、グラフ上で基点ノードとしてのみ表示
        return;
      }

      if (!laneMap.has(base)) {
        const baseNode =
          base !== UNKNOWN_BASE ? branchMap.get(base) ?? null : null;
        laneMap.set(base, {
          id: base,
          baseLabel: base === UNKNOWN_BASE ? "ベース不明" : base,
          baseNode,
          nodes: [],
          isSyntheticBase: baseNode === null,
        });
      }

      laneMap.get(base)!.nodes.push(branch);
    });

    return Array.from(laneMap.values()).sort((a, b) => {
      if (a.id === UNKNOWN_BASE) {
        return 1;
      }
      if (b.id === UNKNOWN_BASE) {
        return -1;
      }
      return a.baseLabel.localeCompare(b.baseLabel, "ja");
    });
  }, [branches, branchMap, referencedBases]);

  if (!lanes.length) {
    return (
      <section className="branch-graph-panel">
        <div className="branch-graph-panel__empty">
          <p>グラフ表示できるブランチがありません。</p>
          <p>fetch済みのブランチやWorktreeを追加すると関係図が表示されます。</p>
        </div>
      </section>
    );
  }

  return (
    <section className="branch-graph-panel">
      <header className="branch-graph-panel__header">
        <div>
          <p className="branch-graph-panel__eyebrow">BRANCH GRAPH</p>
          <h2>ベースブランチの関係をグラフィカルに把握</h2>
          <p>
            baseRef、Git upstream、merge-baseヒューリスティクスを用いて推定したベースブランチ単位で
            派生ノードをレーン表示します。
          </p>
        </div>
        <div className="branch-graph-panel__legend">
          <span className="graph-chip graph-chip--base">Base</span>
          <span className="graph-chip graph-chip--local">Local</span>
          <span className="graph-chip graph-chip--remote">Remote</span>
          <span className="graph-chip graph-chip--worktree">Worktree</span>
        </div>
      </header>

      <div className="branch-graph">
        {lanes.map((lane) => (
          <article className="branch-graph__lane" key={lane.id}>
            <div className="branch-graph__lane-heading">
              <p className="branch-graph__lane-label">
                {lane.baseLabel}
                {lane.baseNode && (
                  <span className="branch-graph__lane-meta">
                    {lane.baseNode.type === "local" ? "LOCAL" : "REMOTE"}
                  </span>
                )}
                {lane.isSyntheticBase && (
                  <span className="branch-graph__lane-meta lane-meta--muted">
                    推定のみ
                  </span>
                )}
              </p>
              <span className="branch-graph__lane-count">
                {lane.nodes.length} branch
                {lane.nodes.length > 1 ? "es" : ""}
              </span>
            </div>

            <div className="branch-graph__track">
              {renderBaseNode(lane)}
              {lane.nodes.map((branch) => (
                <BranchNode key={branch.name} branch={branch} />
              ))}
            </div>
          </article>
        ))}
      </div>
    </section>
  );
}

function renderBaseNode(lane: Lane) {
  const label =
    lane.baseLabel === "ベース不明" ? "Unknown base" : lane.baseLabel;
  const content = (
    <div
      className={`branch-graph__node branch-graph__node--base ${
        lane.baseNode ? `branch-graph__node--${lane.baseNode.type}` : ""
      }`}
    >
      <span className="branch-graph__node-label">{label}</span>
      <span className="branch-graph__node-meta">BASE</span>
      <div className="branch-graph__tooltip">
        <p>{label}</p>
        <p>
          {lane.baseNode
            ? `type: ${lane.baseNode.type}`
            : "推定されたベースブランチ"}
        </p>
      </div>
    </div>
  );

  if (lane.baseNode) {
    return (
      <Link
        key={`base-${lane.id}`}
        to={`/${encodeURIComponent(lane.baseNode.name)}`}
        className="branch-graph__node-link"
        aria-label={`ベースブランチ ${lane.baseNode.name} を開く`}
      >
        {content}
      </Link>
    );
  }

  return (
    <div key={`base-${lane.id}`} className="branch-graph__node-link">
      {content}
    </div>
  );
}

function BranchNode({ branch }: { branch: Branch }) {
  const node = (
    <div
      className={`branch-graph__node branch-graph__node--${branch.type} ${
        branch.mergeStatus === "merged"
          ? "branch-graph__node--merged"
          : branch.mergeStatus === "unmerged"
            ? "branch-graph__node--active"
            : ""
      }`}
    >
      <span className="branch-graph__node-label">
        {formatBranchLabel(branch)}
      </span>
      <span className="branch-graph__node-meta">
        {branch.worktreePath ? "Worktree" : "No Worktree"}
      </span>
      <div className="branch-graph__tooltip">
        <p>{branch.name}</p>
        <p>base: {branch.baseBranch ?? "unknown"}</p>
        <p>{getDivergenceLabel(branch)}</p>
        <p>{branch.worktreePath ?? "Worktree未作成"}</p>
      </div>
    </div>
  );

  return (
    <Link
      to={`/${encodeURIComponent(branch.name)}`}
      className="branch-graph__node-link"
      aria-label={`${branch.name} の詳細を開く`}
    >
      {node}
    </Link>
  );
}
