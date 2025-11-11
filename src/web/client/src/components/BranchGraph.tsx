import React, { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { forceSimulation, forceManyBody, forceCollide, forceX, forceY } from "d3-force";
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

interface ForceNode extends RadialNodeDescriptor {
  x: number;
  y: number;
  vx?: number;
  vy?: number;
}

type Coordinates = { x: number; y: number };

type DivergenceFilter = "ahead" | "behind" | "upToDate";

const PRIMARY_BASES = ["main", "origin/main", "develop", "origin/develop"] as const;
const DIVERGENCE_FILTERS: { id: DivergenceFilter; label: string }[] = [
  { id: "ahead", label: "Ahead" },
  { id: "behind", label: "Behind" },
  { id: "upToDate", label: "Up-to-date" },
];
const NODE_WIDTH_PX = 170;
const MIN_PRIMARY_RADIUS = 200;
const MAX_PRIMARY_RADIUS = 360;
const SECONDARY_RING_OFFSET = 100;

function canonicalName(name?: string | null): string | null {
  if (!name) {
    return null;
  }
  return name.replace(/^origin\//, "");
}

function formatBranchLabel(branch: Branch): string {
  const canonical = canonicalName(branch.name);
  const label = canonical ?? branch.name;
  return label.length > 32
    ? `${label.slice(0, 29)}...`
    : label;
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
  const [nodePositions, setNodePositions] = useState<Partial<Record<string, Coordinates>>>({});
  const [layoutPositions, setLayoutPositions] = useState<Partial<Record<string, Coordinates>>>({});
  const [draggingNode, setDraggingNode] = useState<string | null>(null);
  const normalizedBranches = useMemo(() => {
    const map = new Map<string, Branch>();
    const shouldReplace = (existing: Branch, candidate: Branch) => {
      const existingHasWorktree = Boolean(existing.worktreePath);
      const candidateHasWorktree = Boolean(candidate.worktreePath);
      if (!existingHasWorktree && candidateHasWorktree) {
        return true;
      }
      if (existingHasWorktree && !candidateHasWorktree) {
        return false;
      }
      if (existing.type === "remote" && candidate.type !== "remote") {
        return true;
      }
      if (existing.type !== "remote" && candidate.type === "remote") {
        return false;
      }
      const existingDate = existing.commitDate ?? "";
      const candidateDate = candidate.commitDate ?? "";
      return candidateDate.localeCompare(existingDate) > 0;
    };

    branches.forEach((branch) => {
      const key = canonicalName(branch.name) ?? branch.name;
      const existing = map.get(key);
      if (!existing || shouldReplace(existing, branch)) {
        map.set(key, branch);
      }
    });
    return Array.from(map.values());
  }, [branches]);

  const branchMap = useMemo(() => {
    return new Map(
      normalizedBranches.map((branch) => [canonicalName(branch.name) ?? branch.name, branch]),
    );
  }, [normalizedBranches]);

  const referencedBases = useMemo(() => {
    const baseSet = new Set<string>();
    normalizedBranches.forEach((branch) => {
      const canonicalBase = canonicalName(branch.baseBranch);
      if (canonicalBase) {
        baseSet.add(canonicalBase);
      }
    });
    return baseSet;
  }, [normalizedBranches]);

  const { centerNodes, radialNodes, primaryRadius, secondaryRadius } = useMemo(() => {
    const centers: CenterNodeDescriptor[] = [];
    const centerIds = new Set<string>();

    PRIMARY_BASES.forEach((base) => {
      const canonical = canonicalName(base) ?? base;
      if (centerIds.has(canonical)) {
        return;
      }
      const branch = branchMap.get(canonical) ?? null;
      if (branch || referencedBases.has(canonical)) {
        centers.push({
          id: canonical,
          label: canonical,
          branch,
          isSynthetic: !branch,
        });
        centerIds.add(canonical);
      }
    });

    if (!centers.length && normalizedBranches.length) {
      const fallback =
        normalizedBranches.find((branch) => /main|develop/i.test(branch.name)) ?? normalizedBranches[0];
      if (fallback) {
        const canonical = canonicalName(fallback.name) ?? fallback.name;
        if (!centerIds.has(canonical)) {
          centers.push({
          id: canonical,
          label: canonical,
          branch: fallback,
          isSynthetic: false,
        });
          centerIds.add(canonical);
        }
      }
    }

    const centerNames = new Set(centers.map((center) => center.label));
    const orbitalBranches = normalizedBranches.filter((branch) => {
      const canonical = canonicalName(branch.name) ?? branch.name;
      return !centerNames.has(canonical);
    });

    const grouped = orbitalBranches.reduce<Map<string, Branch[]>>((map, branch) => {
      const canonicalBase = canonicalName(branch.baseBranch);
      const base = canonicalBase ?? (branch.baseBranch ? branch.baseBranch : UNKNOWN_BASE);
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
  }, [normalizedBranches, branchMap, referencedBases]);

  useEffect(() => {
    setNodePositions((prev) => {
      const next: Partial<Record<string, Coordinates>> = {};
      radialNodes.forEach((node) => {
        const existing = prev[node.branch.name];
        if (existing) {
          next[node.branch.name] = existing;
        }
      });
      return next;
    });
  }, [radialNodes]);

  useEffect(() => {
    if (!radialNodes.length) {
      setLayoutPositions({});
      return;
    }

    const nodes: ForceNode[] = radialNodes.map((node) => {
      const coords = polarToCartesian(node.angle, node.radius);
      return { ...node, x: coords.x, y: coords.y };
    });

    const simulation = forceSimulation(nodes)
      .force("charge", forceManyBody().strength(-150))
      .force("collide", forceCollide(NODE_WIDTH_PX * 0.65))
      .force(
        "x",
        forceX<ForceNode>((d) => polarToCartesian(d.angle, d.radius).x).strength(0.12),
      )
      .force(
        "y",
        forceY<ForceNode>((d) => polarToCartesian(d.angle, d.radius).y).strength(0.12),
      )
      .alpha(1)
      .alphaDecay(0.14)
      .velocityDecay(0.35);

    for (let i = 0; i < 160; i += 1) {
      simulation.tick();
    }

    const nextPositions: Partial<Record<string, Coordinates>> = {};
    nodes.forEach((node) => {
      const fallback = polarToCartesian(node.angle, node.radius);
      nextPositions[node.branch.name] = {
        x: node.x ?? fallback.x,
        y: node.y ?? fallback.y,
      };
    });

    setLayoutPositions(nextPositions);

    return () => {
      simulation.stop();
    };
  }, [radialNodes]);

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

  const centerPositions = useMemo(() => {
    if (!centerNodes.length) {
      return new Map<string, Coordinates>();
    }
    const radius = Math.max(coreSize / 2 - 40, 48);
    return new Map<string, Coordinates>(
      centerNodes.map((center, index) => {
        const angle = (index / centerNodes.length) * 2 * Math.PI;
        return [
          center.label,
          {
            x: radius * Math.cos(angle),
            y: radius * Math.sin(angle),
          },
        ];
      }),
    );
  }, [centerNodes, coreSize]);

  const baseFilters = useMemo(() => {
    const labels = new Set<string>();
    radialNodes.forEach((node) => {
      if (node.baseLabel !== "detached") {
        labels.add(node.baseLabel);
      }
    });
    return Array.from(labels).sort((a, b) => a.localeCompare(b, "ja"));
  }, [radialNodes]);

  if (!normalizedBranches.length) {
    return (
      <section className="branch-graph-panel">
        <div className="branch-graph-panel__empty">
          <p>No branches to visualize yet.</p>
          <p>Fetch branches or create worktrees to see relationships here.</p>
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
        <div className="branch-graph-panel__legend">
          <span className="graph-chip graph-chip--base">Base</span>
          <span className="graph-chip graph-chip--local">Local</span>
          <span className="graph-chip graph-chip--remote">Remote</span>
          <span className="graph-chip graph-chip--worktree">Worktree</span>
        </div>
        {baseFilters.length > 0 && (
          <div className="branch-graph__filters" role="group" aria-label="Base branch filters">
            <button
              type="button"
              className={`branch-graph__filter ${!activeBase ? "is-active" : ""}`}
              onClick={() => handleBaseChipClick(null)}
              aria-pressed={!activeBase}
              aria-label="Show all bases"
            >
              All bases
            </button>
            {baseFilters.map((base) => (
              <button
                key={`base-filter-${base}`}
                type="button"
                className={`branch-graph__filter ${activeBase === base ? "is-active" : ""}`}
                onClick={() => handleBaseChipClick(activeBase === base ? null : base)}
                aria-pressed={activeBase === base}
                aria-label={`Focus on ${base}`}
              >
                {base}
              </button>
            ))}
          </div>
        )}
        <div className="branch-graph__filters" role="group" aria-label="Divergence filters">
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
              const fallbackPosition = polarToCartesian(node.angle, node.radius);
              const resolvedPosition =
                nodePositions[node.branch.name] ??
                layoutPositions[node.branch.name] ??
                fallbackPosition;
              const center = orbitSize / 2;
              const baseCenter = baseCenterPosition(centerPositions, node.baseLabel, center);
              const dimmed =
                Boolean(activeBase && node.baseLabel !== activeBase) ||
                Boolean(activeDivergence && !matchesDivergence(node.branch, activeDivergence));
              return (
                <line
                  key={`line-${node.branch.name}`}
                  x1={baseCenter.x}
                  y1={baseCenter.y}
                  x2={center + resolvedPosition.x}
                  y2={center + resolvedPosition.y}
                  className={`radial-graph__connection ${dimmed ? "is-dimmed" : ""}`}
                  strokeLinecap="round"
                />
              );
            })}
          </svg>
          {radialNodes.map((node) => {
            const fallbackPosition = polarToCartesian(node.angle, node.radius);
            const resolvedPosition =
              nodePositions[node.branch.name] ??
              layoutPositions[node.branch.name] ??
              fallbackPosition;
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
              <p>No derived branches yet.</p>
              <p>Branches other than main/develop will appear around this orbit.</p>
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
              position={centerPositions.get(center.label)}
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
  position,
}: {
  descriptor: CenterNodeDescriptor;
  isHighlighted?: boolean;
  position?: Coordinates | undefined;
}) {
  const style = position
    ? {
        left: `calc(50% + ${position.x}px)`,
        top: `calc(50% + ${position.y}px)`,
      }
    : undefined;
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
          View sessions
        </Link>
      )}
    </div>
  );

  const slotClass = `radial-core__slot${descriptor.branch ? "" : " radial-core__slot--synthetic"}`;
  return (
    <div className={slotClass} style={style}>
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
  position?: Coordinates;
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
      aria-label={`Select ${node.branch.name}`}
      onClick={handleSelect}
      onKeyDown={handleKeyDown}
      onPointerDown={(event) => onPointerDown?.(node.branch.name, event)}
      data-base-label={node.baseLabel}
    >
      <div className="radial-node__content">
        <span className="radial-node__label">{formatBranchLabel(node.branch)}</span>
        <span className="radial-node__meta">
          {canonicalName(node.branch.baseBranch) ?? node.branch.baseBranch ?? "Base unknown"}
        </span>
        <div className="radial-node__tooltip">
          <p>{node.branch.name}</p>
          <p>{getDivergenceLabel(node.branch)}</p>
          <p>{node.branch.worktreePath ?? "Worktree missing"}</p>
          <Link
            to={detailLink}
            className="radial-node__detail-link"
            onClick={(event) => event.stopPropagation()}
          >
            Open details
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

function polarToCartesian(angleDeg: number, radius: number): Coordinates {
  const radians = (angleDeg * Math.PI) / 180;
  return {
    x: radius * Math.cos(radians),
    y: radius * Math.sin(radians),
  };
}

function baseCenterPosition(
  positions: Map<string, Coordinates>,
  baseLabel: string,
  fallback: number,
): { x: number; y: number } {
  const position = positions.get(baseLabel);
  if (!position) {
    return { x: fallback, y: fallback };
  }
  return {
    x: fallback + position.x,
    y: fallback + position.y,
  };
}
