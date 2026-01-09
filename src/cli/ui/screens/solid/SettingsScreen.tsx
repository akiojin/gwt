/** @jsxImportSource @opentui/solid */
import { SelectorScreen } from "./SelectorScreen.js";

export interface SettingsItem {
  label: string;
  value: string;
  description?: string;
}

export interface SettingsScreenProps {
  settings: SettingsItem[];
  onSelect?: (setting: SettingsItem) => void;
  onBack?: () => void;
  version?: string | null;
  helpVisible?: boolean;
}

export function SettingsScreen({
  settings,
  onSelect,
  onBack,
  version,
  helpVisible = false,
}: SettingsScreenProps) {
  const items = settings.map((setting) => ({
    label: setting.label,
    value: setting.value,
    ...(setting.description !== undefined
      ? { description: setting.description }
      : {}),
  }));

  const handleSelect = (item: { label: string; value: string }) => {
    const setting = settings.find((entry) => entry.value === item.value);
    if (setting) {
      onSelect?.(setting);
    }
  };

  return (
    <SelectorScreen
      title="gwt - Settings"
      items={items}
      onSelect={handleSelect}
      {...(onBack ? { onBack } : {})}
      {...(version !== undefined ? { version } : {})}
      emptyMessage="No settings available."
      showDescription
      helpVisible={helpVisible}
    />
  );
}
