import { TextAttributes } from "@opentui/core";
import type { JSX } from "solid-js";
import type { BranchViewMode, Statistics } from "../../types.js";

export interface StatsProps {
  stats: Statistics;
  separator?: string;
  lastUpdated?: Date | null;
  viewMode?: BranchViewMode;
}

const formatRelativeTime = (date: Date): string => {
  const now = new Date();
  const diffMs = now.getTime() - date.getTime();
  const diffSec = Math.floor(diffMs / 1000);

  if (diffSec < 60) {
    return `${diffSec}s ago`;
  }

  const diffMin = Math.floor(diffSec / 60);
  if (diffMin < 60) {
    return `${diffMin}m ago`;
  }

  const diffHour = Math.floor(diffMin / 60);
  return `${diffHour}h ago`;
};

const formatViewModeLabel = (mode: BranchViewMode): string => {
  switch (mode) {
    case "all":
      return "All";
    case "local":
      return "Local";
    case "remote":
      return "Remote";
  }
};

export function Stats({
  stats,
  separator = "  ",
  lastUpdated = null,
  viewMode,
}: StatsProps) {
  const segments: JSX.Element[] = [];
  const appendSeparator = () => {
    segments.push(<span attributes={TextAttributes.DIM}>{separator}</span>);
  };

  if (viewMode) {
    segments.push(<span attributes={TextAttributes.DIM}>Mode: </span>);
    segments.push(
      <span fg="white" attributes={TextAttributes.BOLD}>
        {formatViewModeLabel(viewMode)}
      </span>,
    );
    if (stats || lastUpdated) {
      appendSeparator();
    }
  }

  const items = [
    { label: "Local", value: stats.localCount, color: "cyan" },
    { label: "Remote", value: stats.remoteCount, color: "green" },
    { label: "Worktrees", value: stats.worktreeCount, color: "yellow" },
    { label: "Changes", value: stats.changesCount, color: "magenta" },
  ];

  items.forEach((item, index) => {
    segments.push(
      <span attributes={TextAttributes.DIM}>{`${item.label}: `}</span>,
    );
    segments.push(
      <span fg={item.color} attributes={TextAttributes.BOLD}>
        {item.value}
      </span>,
    );
    if (index < items.length - 1 || lastUpdated) {
      appendSeparator();
    }
  });

  if (lastUpdated) {
    segments.push(<span attributes={TextAttributes.DIM}>Updated: </span>);
    segments.push(<span fg="gray">{formatRelativeTime(lastUpdated)}</span>);
  }

  return <text>{segments}</text>;
}
