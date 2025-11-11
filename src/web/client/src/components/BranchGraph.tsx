import React, { useCallback, useMemo } from "react";
import { Link } from "react-router-dom";
import type { Branch } from "../../../../types/api.js";

const UNKNOWN_BASE = "__unknown__";

interface BranchGraphProps {
  branches: Branch[];
  onSelectBranch?: (branch: Branch) => void;
  activeBase?: string | null;
  onBaseFilterChange?: (base: string | null) => void;
}

interface CenterNodeDescriptor {
  id: string;
  label: string;
  branch: Branch | null;
  isSynthetic: boolean;
}

interface RadialNodeDescriptor {
  branch: Branch;
  angle: number;
  radius: number;
  baseLabel: string;
  isPrimaryOrbit: boolean;
}

const PRIMARY_BASES = ["main", "origin/main", "develop", "origin/develop"] as const;

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

export function BranchGraph({
  branches,
  onSelectBranch,
  activeBase,
  onBaseFilterChange,
}: BranchGraphProps) {
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

  const { centerNodes, radialNodes } = useMemo(() => {
    const centers: CenterNodeDescriptor[] = [];

    PRIMARY_BASES.forEach((base) => {
      const branch = branchMap.get(base) ?? null;
      if (branch || referencedBases.has(base)) {
        centers.push({
          id: base,
          label: base,
          branch,
          isSynthetic: !branch,
        });
      }
    });

    if (!centers.length && branches.length) {
      const fallback =
        branches.find((branch) => /main|develop/i.test(branch.name)) ?? branches[0];
      if (fallback) {
        centers.push({
          id: fallback.name,
          label: fallback.name,
          branch: fallback,
          isSynthetic: false,
        });
      }
    }

    const centerNames = new Set(centers.map((center) => center.branch?.name ?? center.label));
    const orbitalBranches = branches.filter((branch) => !centerNames.has(branch.name));

    const grouped = orbitalBranches.reduce<Map<string, Branch[]>>((map, branch) => {
      const base = branch.baseBranch ?? UNKNOWN_BASE;
      if (!map.has(base)) {
        map.set(base, []);
      }
      map.get(base)!.push(branch);
      return map;
    }, new Map());

    const orderedGroups = Array.from(grouped.entries()).sort((a, b) =>
      a[0].localeCompare(b[0], "ja"),
    );

    const totalOrbitalNodes = orbitalBranches.length || 1;
    let cursor = 0;

    const radial: RadialNodeDescriptor[] = [];

    orderedGroups.forEach(([base, nodes]) => {
      nodes.forEach((branch) => {
        const angle = (360 / totalOrbitalNodes) * cursor;
        cursor += 1;
        const isPrimaryOrbit = centerNames.has(base);
        const radius = isPrimaryOrbit ? 180 : 250;
        radial.push({
          branch,
          angle,
          radius,
          baseLabel: base === UNKNOWN_BASE ? "detached" : base,
          isPrimaryOrbit,
        });
      });
    });

    return { centerNodes: centers, radialNodes: radial };
  }, [branches, branchMap, referencedBases]);

  const baseFilters = useMemo(() => {
    const labels = new Set<string>();
    radialNodes.forEach((node) => {
      if (node.baseLabel !== "detached") {
        labels.add(node.baseLabel);
      }
    });
    return Array.from(labels).sort((a, b) => a.localeCompare(b, "ja"));
  }, [radialNodes]);

  if (!branches.length) {
    return (
      <section className="branch-graph-panel">
        <div className="branch-graph-panel__empty">
          <p>グラフ表示できるブランチがありません。</p>
          <p>fetch済みのブランチやWorktreeを追加すると関係図が表示されます。</p>
        </div>
      </section>
    );
  }

  const handleNodeSelect = useCallback(
    (branch: Branch) => {
      onSelectBranch?.(branch);
    },
    [onSelectBranch],
  );

  const handleBaseChipClick = useCallback(
    (base: string | null) => {
      onBaseFilterChange?.(base);
    },
    [onBaseFilterChange],
  );

  return (
    <section className="branch-graph-panel">
      <header className="branch-graph-panel__header">
        <div>
          <p className="branch-graph-panel__eyebrow">BRANCH GRAPH</p>
          <h2>ベースブランチ中心のラジアルビュー</h2>
          <p>
            main / develop を中心に、派生ブランチを放射状に配置したダッシュボードです。ノードを
            クリックすると AI ツールの起動モーダルが開き、詳細リンクからセッション画面へ遷移できます。
          </p>
        </div>
        <div className="branch-graph-panel__legend">
          <span className="graph-chip graph-chip--base">Base</span>
          <span className="graph-chip graph-chip--local">Local</span>
          <span className="graph-chip graph-chip--remote">Remote</span>
          <span className="graph-chip graph-chip--worktree">Worktree</span>
        </div>
        {baseFilters.length > 0 && (
          <div className="branch-graph__filters" role="group" aria-label="ベースブランチのフィルター">
            <button
              type="button"
              className={`branch-graph__filter ${!activeBase ? "is-active" : ""}`}
              onClick={() => handleBaseChipClick(null)}
              aria-pressed={!activeBase}
              aria-label="すべてのベースを表示"
            >
              すべて
            </button>
            {baseFilters.map((base) => (
              <button
                key={base}
                type="button"
                className={`branch-graph__filter ${activeBase === base ? "is-active" : ""}`}
                onClick={() => handleBaseChipClick(activeBase === base ? null : base)}
                aria-pressed={activeBase === base}
                aria-label={`${base} を中心に表示`}
              >
                {base}
              </button>
            ))}
          </div>
        )}
      </header>

      <div className="radial-graph">
        <div className="radial-graph__orbit">
          {radialNodes.map((node) => (
            <RadialBranchNode
              key={node.branch.name}
              node={node}
              isDimmed={Boolean(activeBase && node.baseLabel !== activeBase)}
              onSelect={handleNodeSelect}
            />
          ))}
          {!radialNodes.length && (
            <div className="radial-graph__empty-hint">
              <p>派生ブランチがまだありません。</p>
              <p>main / develop 以外のブランチが追加されると外周に表示されます。</p>
            </div>
          )}
        </div>
        <div className="radial-graph__core">
          {centerNodes.map((center) => (
            <CoreNode
              key={center.id}
              descriptor={center}
              isHighlighted={activeBase ? center.label === activeBase : false}
            />
          ))}
        </div>
      </div>
    </section>
  );
}

function CoreNode({
  descriptor,
  isHighlighted,
}: {
  descriptor: CenterNodeDescriptor;
  isHighlighted?: boolean;
}) {
  const content = (
    <div
      className={`radial-core__node ${
        descriptor.branch ? `radial-core__node--${descriptor.branch.type}` : ""
      } ${isHighlighted ? "radial-core__node--active" : ""}`}
    >
      <p className="radial-core__label">{descriptor.label}</p>
      <p className="radial-core__meta">
        {descriptor.branch ? descriptor.branch.type.toUpperCase() : "ESTIMATED"}
      </p>
      {descriptor.branch && (
        <Link
          to={`/${encodeURIComponent(descriptor.branch.name)}`}
          className="radial-core__link"
        >
          セッションを表示
        </Link>
      )}
    </div>
  );

  if (descriptor.branch) {
    return (
      <div key={descriptor.id} className="radial-core__slot">
        {content}
      </div>
    );
  }

  return (
    <div key={descriptor.id} className="radial-core__slot radial-core__slot--synthetic">
      {content}
    </div>
  );
}

function RadialBranchNode({
  node,
  onSelect,
  isDimmed,
}: {
  node: RadialNodeDescriptor;
  onSelect?: (branch: Branch) => void;
  isDimmed?: boolean;
}) {
  const handleSelect = () => {
    onSelect?.(node.branch);
  };

  const handleKeyDown = (event: React.KeyboardEvent<HTMLDivElement>) => {
    if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      handleSelect();
    }
  };

  const detailLink = `/${encodeURIComponent(node.branch.name)}`;
  const transform = {
    transform: `rotate(${node.angle}deg) translate(${node.radius}px)`
  } as React.CSSProperties;
  const contentRotation = {
    transform: `rotate(${-node.angle}deg)`
  } as React.CSSProperties;

  return (
    <div
      className={`radial-node radial-node--${node.branch.type} ${
        node.branch.worktreePath ? "radial-node--worktree" : ""
      } ${node.isPrimaryOrbit ? "radial-node--primary" : "radial-node--secondary"} ${
        isDimmed ? "radial-node--dimmed" : ""
      }`}
      style={transform}
      role="button"
      tabIndex={0}
      aria-label={`${node.branch.name} を選択`}
      onClick={handleSelect}
      onKeyDown={handleKeyDown}
      data-base-label={node.baseLabel}
    >
      <div className="radial-node__content" style={contentRotation}>
        <span className="radial-node__label">{formatBranchLabel(node.branch)}</span>
        <span className="radial-node__meta">
          {node.branch.baseBranch ?? "ベース不明"}
        </span>
        <div className="radial-node__tooltip">
          <p>{node.branch.name}</p>
          <p>{getDivergenceLabel(node.branch)}</p>
          <p>{node.branch.worktreePath ?? "Worktree未作成"}</p>
          <Link
            to={detailLink}
            className="radial-node__detail-link"
            onClick={(event) => event.stopPropagation()}
          >
            詳細を開く
          </Link>
        </div>
      </div>
    </div>
  );
}
