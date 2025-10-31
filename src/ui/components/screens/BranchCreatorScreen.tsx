import React, { useState, useCallback } from 'react';
import { Box, Text, useInput } from 'ink';
import { Header } from '../parts/Header.js';
import { Footer } from '../parts/Footer.js';
import { Select } from '../common/Select.js';
import { Input } from '../common/Input.js';
import { useTerminalSize } from '../../hooks/useTerminalSize.js';
import { BRANCH_PREFIXES } from '../../../config/constants.js';

type BranchType = 'feature' | 'hotfix' | 'release';
type Step = 'type-selection' | 'name-input';

export interface BranchCreatorScreenProps {
  onBack: () => void;
  onCreate: (branchName: string) => void;
  baseBranch?: string;
  version?: string | null;
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
export function BranchCreatorScreen({ onBack, onCreate, baseBranch, version }: BranchCreatorScreenProps) {
  const { rows } = useTerminalSize();
  const [step, setStep] = useState<Step>('type-selection');
  const [selectedType, setSelectedType] = useState<BranchType>('feature');
  const [branchName, setBranchName] = useState('');

  // Handle keyboard input for back navigation
  useInput((input, key) => {
    if (key.escape) {
      onBack();
    }
  });

  // Branch type options
  const branchTypeItems: BranchTypeItem[] = [
    {
      label: 'feature',
      value: 'feature',
      description: 'New feature development',
    },
    {
      label: 'hotfix',
      value: 'hotfix',
      description: 'Critical bug fix',
    },
    {
      label: 'release',
      value: 'release',
      description: 'Release preparation',
    },
  ];

  // Handle branch type selection
  const handleTypeSelect = useCallback((item: BranchTypeItem) => {
    setSelectedType(item.value);
    setStep('name-input');
  }, []);

  // Handle branch name input
  const handleNameChange = useCallback((value: string) => {
    setBranchName(value);
  }, []);

  // Handle branch creation
  const handleCreate = useCallback(() => {
    if (branchName.trim()) {
      const prefix = BRANCH_PREFIXES[selectedType.toUpperCase() as keyof typeof BRANCH_PREFIXES];
      const fullBranchName = `${prefix}${branchName.trim()}`;
      onCreate(fullBranchName);
    }
  }, [branchName, selectedType, onCreate]);

  // Footer actions
  const footerActions =
    step === 'type-selection'
      ? [
          { key: 'enter', description: 'Select' },
          { key: 'esc', description: 'Back' },
        ]
      : [
          { key: 'enter', description: 'Create' },
          { key: 'esc', description: 'Back' },
        ];

  return (
    <Box flexDirection="column" height={rows}>
      {/* Header */}
      <Header title="New Branch" titleColor="green" version={version} />

      {/* Content */}
      <Box flexDirection="column" flexGrow={1} marginTop={1}>
        {baseBranch && (
          <Box marginBottom={1}>
            <Text>
              Base branch: <Text bold color="cyan">{baseBranch}</Text>
            </Text>
          </Box>
        )}
        {step === 'type-selection' ? (
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
                Branch name prefix: <Text bold>{BRANCH_PREFIXES[selectedType.toUpperCase() as keyof typeof BRANCH_PREFIXES]}</Text>
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
