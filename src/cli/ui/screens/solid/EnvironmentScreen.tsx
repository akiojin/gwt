/** @jsxImportSource @opentui/solid */
import { SelectorScreen } from "./SelectorScreen.js";

export interface EnvironmentVariable {
  key: string;
  value: string;
}

export interface EnvironmentScreenProps {
  variables: EnvironmentVariable[];
  onSelect?: (variable: EnvironmentVariable) => void;
  onBack?: () => void;
  version?: string | null;
}

export function EnvironmentScreen({
  variables,
  onSelect,
  onBack,
  version,
}: EnvironmentScreenProps) {
  const items = variables.map((variable) => ({
    label: `${variable.key}=${variable.value}`,
    value: variable.key,
  }));

  const handleSelect = (item: { label: string; value: string }) => {
    const variable = variables.find((entry) => entry.key === item.value);
    if (variable) {
      onSelect?.(variable);
    }
  };

  return (
    <SelectorScreen
      title="gwt - Environment"
      items={items}
      onSelect={handleSelect}
      onBack={onBack}
      version={version}
      emptyMessage="No environment variables."
    />
  );
}
