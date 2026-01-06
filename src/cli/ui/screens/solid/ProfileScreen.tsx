/** @jsxImportSource @opentui/solid */
import { useKeyboard } from "@opentui/solid";
import { createEffect, createSignal } from "solid-js";
import { SelectorScreen } from "./SelectorScreen.js";

export interface ProfileItem {
  name: string;
  displayName?: string;
  isActive?: boolean;
}

export interface ProfileScreenProps {
  profiles: ProfileItem[];
  onSelect?: (profile: ProfileItem) => void;
  onCreate?: () => void;
  onDelete?: (profile: ProfileItem) => void;
  onEdit?: (profile: ProfileItem) => void;
  onBack?: () => void;
  version?: string | null;
  helpVisible?: boolean;
}

export function ProfileScreen({
  profiles,
  onSelect,
  onCreate,
  onDelete,
  onEdit,
  onBack,
  version,
  helpVisible = false,
}: ProfileScreenProps) {
  const [selectedIndex, setSelectedIndex] = createSignal(0);
  const items = profiles.map((profile) => ({
    label: `${profile.displayName ?? profile.name}${profile.isActive ? " (active)" : ""}`,
    value: profile.name,
  }));

  createEffect(() => {
    if (profiles.length === 0) {
      setSelectedIndex(0);
      return;
    }
    setSelectedIndex((prev) =>
      Math.min(Math.max(prev, 0), profiles.length - 1),
    );
  });

  const getSelectedProfile = () => profiles[selectedIndex()] ?? null;

  useKeyboard((key) => {
    if (helpVisible) {
      return;
    }

    if (key.name === "n") {
      onCreate?.();
      return;
    }

    if (key.name === "e") {
      const profile = getSelectedProfile();
      if (profile) {
        onEdit?.(profile);
      }
      return;
    }

    if (key.name === "d") {
      const profile = getSelectedProfile();
      if (profile) {
        onDelete?.(profile);
      }
    }
  });

  const handleSelect = (item: { label: string; value: string }) => {
    const profile = profiles.find((entry) => entry.name === item.value);
    if (profile) {
      onSelect?.(profile);
    }
  };

  return (
    <SelectorScreen
      title="gwt - Profiles"
      description="Enter: Activate | e: Edit | n: New | d: Delete"
      items={items}
      onSelect={handleSelect}
      selectedIndex={selectedIndex()}
      onSelectedIndexChange={setSelectedIndex}
      {...(onBack ? { onBack } : {})}
      {...(version !== undefined ? { version } : {})}
      emptyMessage="No profiles available."
      helpVisible={helpVisible}
    />
  );
}
