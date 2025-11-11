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
        モーダルを閉じる
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

  it("renders summary metrics and branch cards", () => {
    renderPage();

    expect(screen.getByText("総ブランチ数")).toBeInTheDocument();
    expect(screen.getByTestId("metric-total")).toHaveTextContent("3");
    expect(screen.getByTestId("metric-worktrees")).toHaveTextContent("1");
    expect(
      screen.getAllByText("feature/design-refresh").length,
    ).toBeGreaterThanOrEqual(1);
    expect(screen.getByText("未マージ")).toBeInTheDocument();
    expect(screen.getByText("リモート追跡ブランチ")).toBeInTheDocument();
    expect(
      screen.getByText("ベースブランチ中心のラジアルビュー"),
    ).toBeInTheDocument();
    expect(screen.getAllByRole("button", { name: "AIツールを起動" }).length).toBeGreaterThan(0);
  });

  it("filters branches by the search query and shows empty state when unmatched", () => {
    renderPage();

    const input = screen.getByPlaceholderText("ブランチ名やタイプで検索...");
    fireEvent.change(input, { target: { value: "release" } });

    expect(
      screen.getAllByText("release/v1.0.0").length,
    ).toBeGreaterThanOrEqual(1);
    expect(screen.queryByText("feature/design-refresh")).not.toBeInTheDocument();

    fireEvent.change(input, { target: { value: "zzz" } });
    expect(screen.getByText("一致するブランチがありません")).toBeInTheDocument();
  });

  it("opens the launch modal when branch card is selected and closes on demand", () => {
    renderPage();

    const interactiveCard = screen.getByRole("button", {
      name: "feature/design-refresh のAIツールを設定",
    });

    fireEvent.click(interactiveCard);
    expect(screen.getByTestId("ai-tool-modal")).toHaveTextContent(
      "feature/design-refresh",
    );

    fireEvent.click(screen.getByRole("button", { name: "モーダルを閉じる" }));
    expect(screen.queryByTestId("ai-tool-modal")).toBeNull();
  });

  it("supports keyboard selection of branch cards", () => {
    renderPage();

    const interactiveCard = screen.getByRole("button", {
      name: "release/v1.0.0 のAIツールを設定",
    });

    fireEvent.keyDown(interactiveCard, { key: "Enter" });
    expect(screen.getByTestId("ai-tool-modal")).toHaveTextContent(
      "release/v1.0.0",
    );
  });

  it("allows selection directly from the radial branch graph", () => {
    renderPage();

    const radialNode = screen.getByRole("button", {
      name: "hotfix/security を選択",
    });

    fireEvent.click(radialNode);
    expect(screen.getByTestId("ai-tool-modal")).toHaveTextContent("hotfix/security");
  });
});
