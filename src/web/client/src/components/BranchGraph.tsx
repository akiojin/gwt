import React, { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Link } from "react-router-dom";
import type { Branch } from "../../../../types/api.js";

const UNKNOWN_BASE = "__unknown__";

interface BranchGraphProps {
  branches: Branch[];
  onSelectBranch?: (branch: Branch) => void;
  activeBase?: string | null;
  onBaseFilterChange?: (base: string | null) => void;
  activeDivergence?: DivergenceFilter | null;
  onDivergenceFilterChange?: (filter: DivergenceFilter | null) => void;
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

type DivergenceFilter = "ahead" | "behind" | "upToDate";

const PRIMARY_BASES = ["main", "origin/main", "develop", "origin/develop"] as const;
const DIVERGENCE_FILTERS: { id: DivergenceFilter; label: string }[] = [
  { id: "ahead", label: "Ahead" },
  { id: "behind", label: "Behind" },
  { id: "upToDate", label: "最新" },
];
const NODE_WIDTH_PX = 170;
const MIN_PRIMARY_RADIUS = 200;
const MAX_PRIMARY_RADIUS = 360;
const SECONDARY_RING_OFFSET = 100;

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
  activeDivergence,
  onDivergenceFilterChange,
}: BranchGraphProps) {
  const orbitRef = useRef<HTMLDivElement | null>(null);
  const [nodePositions, setNodePositions] = useState<Record<string, { x: number; y: number }>>({});
  const [draggingNode, setDraggingNode] = useState<string | null>(null);
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

  const { centerNodes, radialNodes, primaryRadius, secondaryRadius } = useMemo(() => {
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
    const angleStep = (2 * Math.PI) / totalOrbitalNodes;
    const minRadiusForSpacing = angleStep === 0
      ? MIN_PRIMARY_RADIUS
      : NODE_WIDTH_PX / (2 * Math.sin(angleStep / 2));
    const computedPrimaryRadius = Math.min(
      MAX_PRIMARY_RADIUS,
      Math.max(MIN_PRIMARY_RADIUS, minRadiusForSpacing),
    );
    const computedSecondaryRadius = computedPrimaryRadius + SECONDARY_RING_OFFSET;
    let cursor = 0;

    const radial: RadialNodeDescriptor[] = [];

    orderedGroups.forEach(([base, nodes]) => {
      nodes.forEach((branch) => {
        const angle = (360 / totalOrbitalNodes) * cursor;
        cursor += 1;
        const isPrimaryOrbit = centerNames.has(base);
        const radius = isPrimaryOrbit ? computedPrimaryRadius : computedSecondaryRadius;
        radial.push({
          branch,
          angle,
          radius,
          baseLabel: base === UNKNOWN_BASE ? "detached" : base,
          isPrimaryOrbit,
        });
      });
    });

    return {
      centerNodes: centers,
      radialNodes: radial,
      primaryRadius: computedPrimaryRadius,
      secondaryRadius: computedSecondaryRadius,
    };
  }, [branches, branchMap, referencedBases]);

  const defaultPositions = useMemo(() => {
    const map: Record<string, { x: number; y: number }> = {};
    radialNodes.forEach((node) => {
      const radians = (node.angle * Math.PI) / 180;
      const x = node.radius * Math.cos(radians);
      const y = node.radius * Math.sin(radians);
      map[node.branch.name] = { x, y };
    });
    return map;
  }, [radialNodes]);

  useEffect(() => {
    setNodePositions((prev) => {
      const next: Record<string, { x: number; y: number }> = {};
      radialNodes.forEach((node) => {
        next[node.branch.name] =
          prev[node.branch.name] ?? defaultPositions[node.branch.name] ?? { x: 0, y: 0 };
      });
      return next;
    });
  }, [radialNodes, defaultPositions]);

  const orbitSize = useMemo(() => {
    const radius = secondaryRadius ?? (MIN_PRIMARY_RADIUS + SECONDARY_RING_OFFSET);
    const diameter = (radius + 140) * 2;
    const clamped = Math.min(Math.max(diameter, 480), 960);
    return clamped;
  }, [secondaryRadius]);

  const coreSize = useMemo(() => {
    const base = primaryRadius ?? MIN_PRIMARY_RADIUS;
    return Math.min(Math.max(base * 0.9, 200), 320);
  }, [primaryRadius]);

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

  const updatePositionFromPointer = useCallback(
    (clientX: number, clientY: number, branchName: string) => {
      const orbit = orbitRef.current;
      if (!orbit) {
        return;
      }
      const rect = orbit.getBoundingClientRect();
      const centerX = rect.left + rect.width / 2;
      const centerY = rect.top + rect.height / 2;
      let dx = clientX - centerX;
      let dy = clientY - centerY;
      const maxRadius = Math.max(Math.min(rect.width, rect.height) / 2 - 40, MIN_PRIMARY_RADIUS);
      const distance = Math.sqrt(dx * dx + dy * dy);
      if (distance > maxRadius) {
        const ratio = maxRadius / distance;
        dx *= ratio;
        dy *= ratio;
      }
      setNodePositions((prev) => ({
        ...prev,
        [branchName]: { x: dx, y: dy },
      }));
    },
    [],
  );

  useEffect(() => {
    if (!draggingNode) {
      return undefined;
    }
    const handlePointerMove = (event: PointerEvent) => {
      updatePositionFromPointer(event.clientX, event.clientY, draggingNode);
    };
    const handlePointerUp = () => {
      setDraggingNode(null);
    };
    window.addEventListener("pointermove", handlePointerMove);
    window.addEventListener("pointerup", handlePointerUp);
    return () => {
      window.removeEventListener("pointermove", handlePointerMove);
      window.removeEventListener("pointerup", handlePointerUp);
    };
  }, [draggingNode, updatePositionFromPointer]);

  const handleNodePointerDown = useCallback(
    (branchName: string, event: React.PointerEvent) => {
      event.preventDefault();
      setDraggingNode(branchName);
      updatePositionFromPointer(event.clientX, event.clientY, branchName);
    },
    [updatePositionFromPointer],
  );

  const handleBaseChipClick = useCallback(
    (base: string | null) => {
      onBaseFilterChange?.(base);
    },
    [onBaseFilterChange],
  );

  const handleDivergenceChipClick = useCallback(
    (filter: DivergenceFilter | null) => {
      onDivergenceFilterChange?.(filter);
    },
    [onDivergenceFilterChange],
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
        <div className="branch-graph__filters" role="group" aria-label="差分フィルター">
          <button
            type="button"
            className={`branch-graph__filter ${!activeDivergence ? "is-active" : ""}`}
            onClick={() => handleDivergenceChipClick(null)}
            aria-pressed={!activeDivergence}
          >
            divergence: ALL
          </button>
          {DIVERGENCE_FILTERS.map((option) => (
            <button
              key={option.id}
              type="button"
              className={`branch-graph__filter ${activeDivergence === option.id ? "is-active" : ""}`}
              onClick={() =>
                handleDivergenceChipClick(
                  activeDivergence === option.id ? null : option.id,
                )
              }
              aria-pressed={activeDivergence === option.id}
            >
              {option.label}
            </button>
          ))}
        </div>
      </header>

      <div className="radial-graph">
        <div
          className="radial-graph__orbit"
          style={{ width: `${orbitSize}px`, height: `${orbitSize}px` }}
          ref={orbitRef}
        >
          <svg
            className="radial-graph__connections"
            width={orbitSize}
            height={orbitSize}
            viewBox={`0 0 ${orbitSize} ${orbitSize}`}
          >
            {radialNodes.map((node) => {
              const resolvedPosition =
                nodePositions[node.branch.name] ?? defaultPositions[node.branch.name] ?? { x: 0, y: 0 };
              const center = orbitSize / 2;
              const dimmed =
                Boolean(activeBase && node.baseLabel !== activeBase) ||
                Boolean(activeDivergence && !matchesDivergence(node.branch, activeDivergence));
              return (
                <line
                  key={`line-${node.branch.name}`}
                  x1={center}
                  y1={center}
                  x2={center + resolvedPosition.x}
                  y2={center + resolvedPosition.y}
                  className={`radial-graph__connection ${dimmed ? "is-dimmed" : ""}`}
                  strokeLinecap="round"
                />
              );
            })}
          </svg>
          {radialNodes.map((node) => {
            const resolvedPosition =
              nodePositions[node.branch.name] ?? defaultPositions[node.branch.name] ?? { x: 0, y: 0 };
            return (
              <RadialBranchNode
                key={node.branch.name}
                node={node}
                isDimmed={
                  Boolean(activeBase && node.baseLabel !== activeBase) ||
                  Boolean(activeDivergence && !matchesDivergence(node.branch, activeDivergence))
                }
                position={resolvedPosition}
                onSelect={handleNodeSelect}
                onPointerDown={handleNodePointerDown}
              />
            );
          })}
          {!radialNodes.length && (
            <div className="radial-graph__empty-hint">
              <p>派生ブランチがまだありません。</p>
              <p>main / develop 以外のブランチが追加されると外周に表示されます。</p>
            </div>
          )}
        </div>
        <div
          className="radial-graph__core"
          style={{ width: `${coreSize}px`, height: `${coreSize}px` }}
        >
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
  position,
  onPointerDown,
}: {
  node: RadialNodeDescriptor;
  onSelect?: (branch: Branch) => void;
  isDimmed?: boolean;
  position?: { x: number; y: number };
  onPointerDown?: (branchName: string, event: React.PointerEvent) => void;
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
  const coords = position ?? { x: 0, y: 0 };
  const nodeStyle: React.CSSProperties = {
    left: `calc(50% + ${coords.x}px)`,
    top: `calc(50% + ${coords.y}px)`,
  };

  return (
    <div
      className={`radial-node radial-node--${node.branch.type} ${
        node.branch.worktreePath ? "radial-node--worktree" : ""
      } ${node.isPrimaryOrbit ? "radial-node--primary" : "radial-node--secondary"} ${
        isDimmed ? "radial-node--dimmed" : ""
      }`}
      style={nodeStyle}
      role="button"
      tabIndex={0}
      aria-label={`${node.branch.name} を選択`}
      onClick={handleSelect}
      onKeyDown={handleKeyDown}
      onPointerDown={(event) => onPointerDown?.(node.branch.name, event)}
      data-base-label={node.baseLabel}
    >
      <div className="radial-node__content">
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

function matchesDivergence(branch: Branch, filter: DivergenceFilter): boolean {
  if (!branch.divergence) {
    return false;
  }
  if (filter === "upToDate") {
    return Boolean(branch.divergence.upToDate);
  }
  if (filter === "ahead") {
    return (branch.divergence.ahead ?? 0) > 0;
  }
  if (filter === "behind") {
    return (branch.divergence.behind ?? 0) > 0;
  }
  return false;
}
