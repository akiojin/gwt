/** @jsxImportSource @opentui/solid */
import { SelectorScreen } from "./SelectorScreen.js";

export interface ProfileItem {
  name: string;
  displayName?: string;
  isActive?: boolean;
}

export interface ProfileScreenProps {
  profiles: ProfileItem[];
  onSelect?: (profile: ProfileItem) => void;
  onBack?: () => void;
  version?: string | null;
  helpVisible?: boolean;
}

export function ProfileScreen({
  profiles,
  onSelect,
  onBack,
  version,
  helpVisible = false,
}: ProfileScreenProps) {
  const items = profiles.map((profile) => ({
    label: `${profile.displayName ?? profile.name}${profile.isActive ? " (active)" : ""}`,
    value: profile.name,
  }));

  const handleSelect = (item: { label: string; value: string }) => {
    const profile = profiles.find((entry) => entry.name === item.value);
    if (profile) {
      onSelect?.(profile);
    }
  };

  return (
    <SelectorScreen
      title="gwt - Profiles"
      items={items}
      onSelect={handleSelect}
      onBack={onBack}
      version={version}
      emptyMessage="No profiles available."
      helpVisible={helpVisible}
    />
  );
}
