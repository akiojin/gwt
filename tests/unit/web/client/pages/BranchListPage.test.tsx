import React from "react";
import type { Mock } from "vitest";
import { describe, it, expect, beforeEach, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import type { Branch } from "../../../../../src/types/api.js";
import { BranchListPage } from "../../../../../src/web/client/src/pages/BranchListPage.js";
import { useBranches } from "../../../../../src/web/client/src/hooks/useBranches.js";

vi.mock("../../../../../src/web/client/src/hooks/useBranches.js", () => ({
  useBranches: vi.fn(),
}));

vi.mock("../../../../../src/web/client/src/components/AIToolLaunchModal.tsx", () => ({
  AIToolLaunchModal: ({ branch, onClose }: { branch: Branch; onClose: () => void }) => (
    <div data-testid="ai-tool-modal">
      <p>{branch.name}</p>
      <button type="button" onClick={onClose}>
        Close modal
      </button>
    </div>
  ),
}));

const mockedUseBranches = useBranches as unknown as Mock;

const sampleBranches: Branch[] = [
  {
    name: "feature/design-refresh",
    type: "local",
    mergeStatus: "unmerged",
    worktreePath: "/tmp/feature-design",
     baseBranch: "main",
    commitHash: "abc123",
    commitMessage: "Refine UI layout",
    author: "Akira",
    commitDate: "2025-11-10T09:00:00.000Z",
    divergence: { ahead: 2, behind: 0, upToDate: false },
    hasUnpushedCommits: true,
    prInfo: null,
  },
  {
    name: "release/v1.0.0",
    type: "remote",
    mergeStatus: "merged",
    worktreePath: null,
     baseBranch: "main",
    commitHash: "def789",
    commitMessage: "Tagged release",
    author: "Sana",
    commitDate: "2025-11-05T04:00:00.000Z",
    divergence: { ahead: 0, behind: 0, upToDate: true },
    hasUnpushedCommits: false,
    prInfo: {
      number: 42,
      title: "Release v1.0.0",
      state: "merged",
      mergedAt: "2025-11-04T10:00:00.000Z",
    },
  },
  {
    name: "hotfix/security",
    type: "local",
    mergeStatus: "unknown",
    worktreePath: null,
     baseBranch: "origin/main",
    commitHash: "ghi456",
    commitMessage: "Patch CVE",
    author: "Noa",
    commitDate: "2025-11-03T01:00:00.000Z",
    divergence: { ahead: 0, behind: 3, upToDate: false },
    hasUnpushedCommits: false,
    prInfo: null,
  },
];

describe("BranchListPage", () => {
  beforeEach(() => {
    mockedUseBranches.mockReturnValue({
      data: sampleBranches,
      isLoading: false,
      error: null,
    });
  });

  const renderPage = () =>
    render(
      <MemoryRouter>
        <BranchListPage />
      </MemoryRouter>,
    );

  const switchToListView = () => {
    fireEvent.click(screen.getByRole("button", { name: "List view" }));
  };

  it("renders summary metrics and branch cards", () => {
    renderPage();

    expect(screen.getByText("Total branches")).toBeInTheDocument();
    expect(screen.getByTestId("metric-total")).toHaveTextContent("3");
    expect(screen.getByTestId("metric-worktrees")).toHaveTextContent("1");
    expect(
      screen.getAllByText("feature/design-refresh").length,
    ).toBeGreaterThanOrEqual(1);
    expect(screen.getByText("Remote tracking")).toBeInTheDocument();
    // List view can still be accessed via toggle
    switchToListView();
    expect(screen.getByText("Not merged")).toBeInTheDocument();
    expect(screen.getAllByRole("button", { name: "Launch AI tool" }).length).toBeGreaterThan(0);
  });

  it("filters branches by the search query and shows empty state when unmatched", () => {
    renderPage();

    switchToListView();

    const input = screen.getByPlaceholderText("Search by branch name or type...");
    fireEvent.change(input, { target: { value: "release" } });

    expect(
      screen.getAllByText("release/v1.0.0").length,
    ).toBeGreaterThanOrEqual(1);
    expect(screen.queryByText("feature/design-refresh")).not.toBeInTheDocument();

    fireEvent.change(input, { target: { value: "zzz" } });
    expect(screen.getByText("No branches match your filters")).toBeInTheDocument();
  });

  it("opens the launch modal when branch card is selected and closes on demand", () => {
    renderPage();

    switchToListView();

    const interactiveCard = screen.getByRole("button", {
      name: "Configure AI tool for feature/design-refresh",
    });

    fireEvent.click(interactiveCard);
    expect(screen.getByTestId("ai-tool-modal")).toHaveTextContent(
      "feature/design-refresh",
    );

    fireEvent.click(screen.getByRole("button", { name: "Close modal" }));
    expect(screen.queryByTestId("ai-tool-modal")).toBeNull();
  });

  it("supports keyboard selection of branch cards", () => {
    renderPage();

    switchToListView();

    const interactiveCard = screen.getByRole("button", {
      name: "Configure AI tool for release/v1.0.0",
    });

    fireEvent.keyDown(interactiveCard, { key: "Enter" });
    expect(screen.getByTestId("ai-tool-modal")).toHaveTextContent(
      "release/v1.0.0",
    );
  });

  it("allows selection directly from the radial branch graph", () => {
    renderPage();

    const radialNode = screen.getByRole("button", {
      name: "Select hotfix/security",
    });

    fireEvent.click(radialNode);
    expect(screen.getByTestId("ai-tool-modal")).toHaveTextContent("hotfix/security");
  });

  it("defaults to graph view and toggles to list view when requested", () => {
    renderPage();

    const graphButton = screen.getByRole("button", { name: "Graph view" });
    const listButton = screen.getByRole("button", { name: "List view" });

    expect(graphButton).toHaveAttribute("aria-pressed", "true");
    expect(listButton).toHaveAttribute("aria-pressed", "false");

    fireEvent.click(listButton);
    expect(graphButton).toHaveAttribute("aria-pressed", "false");
    expect(listButton).toHaveAttribute("aria-pressed", "true");
  });

  it("applies base branch filter to both graph and list views", () => {
    renderPage();

    const mainFilter = screen.getByRole("button", { name: "Focus on main" });
    fireEvent.click(mainFilter);

    switchToListView();

    expect(screen.getByText("feature/design-refresh")).toBeInTheDocument();
    expect(screen.getByText("hotfix/security")).toBeInTheDocument();

    const clearFilter = screen.getByRole("button", { name: "Clear base filter main" });
    fireEvent.click(clearFilter);

    expect(screen.getByText("hotfix/security")).toBeInTheDocument();
  });

  it("applies divergence filters and dims unmatched nodes", () => {
    renderPage();

    const aheadFilter = screen.getByRole("button", { name: "Ahead" });
    fireEvent.click(aheadFilter);

    switchToListView();

    expect(screen.getByText("feature/design-refresh")).toBeInTheDocument();
    expect(screen.queryByText("hotfix/security")).toBeNull();
    expect(screen.queryByText("release/v1.0.0")).toBeNull();

    const clearButton = screen.getByRole("button", {
      name: "Clear divergence filter ahead",
    });
    fireEvent.click(clearButton);
    expect(screen.getByText("hotfix/security")).toBeInTheDocument();
  });
});
