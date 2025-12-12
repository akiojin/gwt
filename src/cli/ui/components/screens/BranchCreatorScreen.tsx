import React, { useState, useCallback, useEffect, useRef } from "react";
import { Box, Text, useInput } from "ink";
import { Header } from "../parts/Header.js";
import { Footer } from "../parts/Footer.js";
import { Select } from "../common/Select.js";
import { Input } from "../common/Input.js";
import { useTerminalSize } from "../../hooks/useTerminalSize.js";
import { BRANCH_PREFIXES } from "../../../../config/constants.js";

type BranchType = "feature" | "bugfix" | "hotfix" | "release";
type Step = "type-selection" | "name-input";

const SPINNER_FRAMES = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧"];

export interface BranchCreatorScreenProps {
  onBack: () => void;
  onCreate: (branchName: string) => Promise<void>;
  baseBranch?: string;
  version?: string | null;
  disableAnimation?: boolean;
}

interface BranchTypeItem {
  label: string;
  value: BranchType;
  description: string;
}

/**
 * BranchCreatorScreen - Screen for creating new branches
 * Layout: Header + Type Selection or Name Input + Footer
 * Flow: Type Selection → Name Input → onCreate
 */
export function BranchCreatorScreen({
  onBack,
  onCreate,
  baseBranch,
  version,
  disableAnimation = false,
}: BranchCreatorScreenProps) {
  const { rows } = useTerminalSize();
  const [step, setStep] = useState<Step>("type-selection");
  const [selectedType, setSelectedType] = useState<BranchType>("feature");
  const [branchName, setBranchName] = useState("");
  const [isCreating, setIsCreating] = useState(false);
  const [pendingBranchName, setPendingBranchName] = useState<string | null>(
    null,
  );
  const spinnerIndexRef = useRef(0);
  const [spinnerIndex, setSpinnerIndex] = useState(0);

  const spinnerFrame = SPINNER_FRAMES[spinnerIndex] ?? SPINNER_FRAMES[0];

  // Handle keyboard input for back navigation
  useInput((input, key) => {
    if (isCreating) {
      return;
    }

    if (key.escape) {
      onBack();
    }
  });

  // Branch type options
  const branchTypeItems: BranchTypeItem[] = [
    {
      label: "feature",
      value: "feature",
      description: "New feature development",
    },
    {
      label: "bugfix",
      value: "bugfix",
      description: "Bug fix",
    },
    {
      label: "hotfix",
      value: "hotfix",
      description: "Critical bug fix",
    },
    {
      label: "release",
      value: "release",
      description: "Release preparation",
    },
  ];

  // Handle branch type selection
  const handleTypeSelect = useCallback((item: BranchTypeItem) => {
    setSelectedType(item.value);
    setStep("name-input");
  }, []);

  // Handle branch name input
  const handleNameChange = useCallback(
    (value: string) => {
      if (isCreating) {
        return;
      }
      setBranchName(value);
    },
    [isCreating],
  );

  // Handle branch creation
  const handleCreate = useCallback(async () => {
    if (isCreating) {
      return;
    }

    const trimmedName = branchName.trim();
    if (!trimmedName) {
      return;
    }

    const prefix =
      BRANCH_PREFIXES[
        selectedType.toUpperCase() as keyof typeof BRANCH_PREFIXES
      ];
    const fullBranchName = `${prefix}${trimmedName}`;

    setIsCreating(true);
    setPendingBranchName(fullBranchName);

    try {
      await onCreate(fullBranchName);
    } catch (error) {
      setPendingBranchName(null);
      setIsCreating(false);
      throw error;
    }
  }, [branchName, selectedType, onCreate, isCreating]);

  // Footer actions
  const footerActions = isCreating
    ? []
    : step === "type-selection"
      ? [
          { key: "enter", description: "Select" },
          { key: "esc", description: "Back" },
        ]
      : [
          { key: "enter", description: "Create" },
          { key: "esc", description: "Back" },
        ];

  useEffect(() => {
    if (!isCreating || disableAnimation) {
      spinnerIndexRef.current = 0;
      setSpinnerIndex(0);
      return undefined;
    }

    const interval = setInterval(() => {
      spinnerIndexRef.current =
        (spinnerIndexRef.current + 1) % SPINNER_FRAMES.length;
      setSpinnerIndex(spinnerIndexRef.current);
    }, 120);

    return () => {
      clearInterval(interval);
      spinnerIndexRef.current = 0;
      setSpinnerIndex(0);
    };
  }, [isCreating, disableAnimation]);

  return (
    <Box flexDirection="column" height={rows}>
      {/* Header */}
      <Header title="New Branch" titleColor="green" version={version} />

      {/* Content */}
      <Box flexDirection="column" flexGrow={1} marginTop={1}>
        {baseBranch && (
          <Box marginBottom={1}>
            <Text>
              Base branch:{" "}
              <Text bold color="cyan">
                {baseBranch}
              </Text>
            </Text>
          </Box>
        )}
        {isCreating ? (
          <Box flexDirection="column">
            <Box marginBottom={1}>
              <Text>
                {spinnerFrame}{" "}
                <Text color="cyan">
                  Creating branch{" "}
                  <Text bold>
                    {pendingBranchName ??
                      `${BRANCH_PREFIXES[selectedType.toUpperCase() as keyof typeof BRANCH_PREFIXES]}${branchName.trim()}`}
                  </Text>
                </Text>
              </Text>
            </Box>
            <Text color="gray">
              Please wait while the branch is being created...
            </Text>
          </Box>
        ) : step === "type-selection" ? (
          <Box flexDirection="column">
            <Box marginBottom={1}>
              <Text>Select branch type:</Text>
            </Box>
            <Select items={branchTypeItems} onSelect={handleTypeSelect} />
          </Box>
        ) : (
          <Box flexDirection="column">
            <Box marginBottom={1}>
              <Text>
                Branch name prefix:{" "}
                <Text bold>
                  {
                    BRANCH_PREFIXES[
                      selectedType.toUpperCase() as keyof typeof BRANCH_PREFIXES
                    ]
                  }
                </Text>
              </Text>
            </Box>
            <Input
              value={branchName}
              onChange={handleNameChange}
              onSubmit={handleCreate}
              placeholder="Enter branch name (e.g., add-new-feature)"
            />
          </Box>
        )}
      </Box>

      {/* Footer */}
      <Footer actions={footerActions} />
    </Box>
  );
}
