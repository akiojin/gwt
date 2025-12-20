/**
 * 環境変数プロファイルエディター画面
 *
 * プロファイルの選択・作成・削除・環境変数の編集を行います。
 * @see specs/SPEC-dafff079/spec.md
 */

import React, { useState, useCallback, useMemo, useEffect } from "react";
import { Box, Text } from "ink";
import { Header } from "../parts/Header.js";
import { Footer } from "../parts/Footer.js";
import { Select } from "../common/Select.js";
import { Input } from "../common/Input.js";
import { Confirm } from "../common/Confirm.js";
import { useAppInput } from "../../hooks/useAppInput.js";
import { useTerminalSize } from "../../hooks/useTerminalSize.js";
import { useProfiles } from "../../hooks/useProfiles.js";
import { isValidProfileName } from "../../../../types/profiles.js";

/**
 * Props for `EnvironmentProfileScreen`.
 */
export interface EnvironmentProfileScreenProps {
  onBack: () => void;
  version?: string | null;
}

type ScreenMode =
  | "list" // プロファイル一覧
  | "view" // プロファイル詳細
  | "create-name" // 新規作成：名前入力
  | "create-display" // 新規作成：表示名入力
  | "add-env-key" // 環境変数追加：キー入力
  | "add-env-value" // 環境変数追加：値入力
  | "edit-env-value" // 環境変数編集：値入力
  | "confirm-delete-profile" // プロファイル削除確認
  | "confirm-delete-env"; // 環境変数削除確認

interface ProfileItem {
  label: string;
  value: string;
  profileName: string | null;
  isActive: boolean;
}

interface EnvVarItem {
  label: string;
  value: string;
  key: string;
  envValue: string;
}

const UI_CHROME_HEIGHT = 20; // ヘッダー/フッター/余白などの固定行数
const ENV_VAR_KEY_PATTERN = /^[A-Za-z_][A-Za-z0-9_]*$/;
const NO_PROFILE_VALUE = "__gwt_no_profile__";

type FocusTarget = "profiles" | "env" | "osenv";

interface SelectionState {
  profileIndex: number;
  envIndex: number;
  osEnvIndex: number;
  focus: FocusTarget;
  selectedProfileName: string | null;
}

interface ProfileCreationState {
  name: string;
  displayName: string;
}

interface EnvEditState {
  key: string;
  value: string;
  selectedKey: string | null;
}

type NumberUpdater = number | ((prev: number) => number);
type FocusUpdater = FocusTarget | ((prev: FocusTarget) => FocusTarget);

/**
 * 環境変数プロファイルエディター画面
 */
export function EnvironmentProfileScreen({
  onBack,
  version,
}: EnvironmentProfileScreenProps) {
  const { rows } = useTerminalSize();
  const {
    profiles,
    loading,
    error,
    activeProfileName,
    setActiveProfile,
    createProfile,
    deleteProfile,
    updateEnvVar,
    deleteEnvVar,
  } = useProfiles();

  // 画面モード
  const [mode, setMode] = useState<ScreenMode>("list");

  const [selection, setSelection] = useState<SelectionState>({
    profileIndex: 0,
    envIndex: 0,
    osEnvIndex: 0,
    focus: "profiles",
    selectedProfileName: null,
  });

  const [profileCreation, setProfileCreation] = useState<ProfileCreationState>({
    name: "",
    displayName: "",
  });

  const [envEdit, setEnvEdit] = useState<EnvEditState>({
    key: "",
    value: "",
    selectedKey: null,
  });

  // バリデーションエラー
  const [validationError, setValidationError] = useState<string | null>(null);

  const profileIndex = selection.profileIndex;
  const envIndex = selection.envIndex;
  const osEnvIndex = selection.osEnvIndex;
  const focus = selection.focus;
  const selectedProfileName = selection.selectedProfileName;

  const newProfileName = profileCreation.name;
  const newProfileDisplayName = profileCreation.displayName;

  const newEnvKey = envEdit.key;
  const newEnvValue = envEdit.value;
  const selectedEnvKey = envEdit.selectedKey;

  const setProfileIndex = useCallback((index: number) => {
    setSelection((prev) => ({ ...prev, profileIndex: index }));
  }, []);

  const setEnvIndex = useCallback((updater: NumberUpdater) => {
    setSelection((prev) => ({
      ...prev,
      envIndex:
        typeof updater === "function" ? updater(prev.envIndex) : updater,
    }));
  }, []);

  const setOsEnvIndex = useCallback((updater: NumberUpdater) => {
    setSelection((prev) => ({
      ...prev,
      osEnvIndex:
        typeof updater === "function" ? updater(prev.osEnvIndex) : updater,
    }));
  }, []);

  const setFocus = useCallback((updater: FocusUpdater) => {
    setSelection((prev) => ({
      ...prev,
      focus: typeof updater === "function" ? updater(prev.focus) : updater,
    }));
  }, []);

  const setSelectedProfileName = useCallback((name: string | null) => {
    setSelection((prev) => ({ ...prev, selectedProfileName: name }));
  }, []);

  const setNewProfileName = useCallback((value: string) => {
    setProfileCreation((prev) => ({ ...prev, name: value }));
  }, []);

  const setNewProfileDisplayName = useCallback((value: string) => {
    setProfileCreation((prev) => ({ ...prev, displayName: value }));
  }, []);

  const setNewEnvKey = useCallback((value: string) => {
    setEnvEdit((prev) => ({ ...prev, key: value }));
  }, []);

  const setNewEnvValue = useCallback((value: string) => {
    setEnvEdit((prev) => ({ ...prev, value }));
  }, []);

  const setSelectedEnvKey = useCallback((key: string | null) => {
    setEnvEdit((prev) => ({ ...prev, selectedKey: key }));
  }, []);

  // プロファイル一覧アイテム
  const profileItems: ProfileItem[] = useMemo(() => {
    if (!profiles) return [];
    const items = Object.entries(profiles.profiles).map(([name, profile]) => ({
      label: `${profile.displayName}${name === activeProfileName ? " (active)" : ""}`,
      value: name,
      profileName: name,
      isActive: name === activeProfileName,
    }));
    return [
      {
        label: `(none)${activeProfileName === null ? " (active)" : ""}`,
        value: NO_PROFILE_VALUE,
        profileName: null,
        isActive: activeProfileName === null,
      },
      ...items,
    ];
  }, [profiles, activeProfileName]);

  // 現在選択中のプロファイル
  const currentProfile =
    selectedProfileName && profiles
      ? profiles.profiles[selectedProfileName]
      : null;

  // 環境変数一覧アイテム
  const envItems: EnvVarItem[] = useMemo(() => {
    if (!currentProfile) return [];
    return Object.entries(currentProfile.env).map(([key, value]) => ({
      label: `${key}=${value}`,
      value: key,
      key,
      envValue: value,
    }));
  }, [currentProfile]);

  // OS環境変数（プロファイルで上書きされるものをハイライト）
  const osEnvItems = useMemo(() => {
    const profileEnvKeys = new Set(
      currentProfile ? Object.keys(currentProfile.env) : [],
    );
    return Object.entries(process.env)
      .filter(([, value]) => value !== undefined)
      .map(([key, value]) => ({
        key,
        value: value ?? "",
        isOverwritten: profileEnvKeys.has(key),
      }))
      .sort((a, b) => a.key.localeCompare(b.key));
  }, [currentProfile]);

  // 配列サイズ変更に追従してインデックスをクランプ（削除後の範囲外アクセス防止）
  useEffect(() => {
    if (profileItems.length === 0) {
      if (profileIndex !== 0) {
        setProfileIndex(0);
      }
      return;
    }
    if (profileIndex >= profileItems.length) {
      setProfileIndex(profileItems.length - 1);
    }
  }, [profileItems.length, profileIndex, setProfileIndex]);

  useEffect(() => {
    if (envItems.length === 0) {
      if (envIndex !== 0) {
        setEnvIndex(0);
      }
      return;
    }
    if (envIndex >= envItems.length) {
      setEnvIndex(envItems.length - 1);
    }
  }, [envItems.length, envIndex, setEnvIndex]);

  useEffect(() => {
    if (osEnvItems.length === 0) {
      if (osEnvIndex !== 0) {
        setOsEnvIndex(0);
      }
      return;
    }
    if (osEnvIndex >= osEnvItems.length) {
      setOsEnvIndex(osEnvItems.length - 1);
    }
  }, [osEnvItems.length, osEnvIndex, setOsEnvIndex]);

  // プロファイルを選択してアクティブ化
  const handleActivateProfile = useCallback(
    async (item: ProfileItem) => {
      try {
        await setActiveProfile(item.profileName);
        if (item.profileName === null) {
          setSelectedProfileName(null);
          setFocus("profiles");
          setProfileIndex(0);
          setMode("list");
          return;
        }

        setSelectedProfileName(item.profileName);
        setFocus("env"); // viewモードでは環境変数にフォーカス
        setMode("view");
      } catch {
        // エラー状態は useProfiles フック側で管理するため、ここでは握りつぶす
      }
    },
    [setActiveProfile, setSelectedProfileName, setFocus, setProfileIndex],
  );

  // 新規プロファイル作成開始
  const handleStartCreateProfile = useCallback(() => {
    setNewProfileName("");
    setNewProfileDisplayName("");
    setValidationError(null);
    setMode("create-name");
  }, []);

  // 新規プロファイル名入力完了
  const handleCreateNameSubmit = useCallback((name: string) => {
    if (!isValidProfileName(name)) {
      setValidationError(
        "Invalid profile name. Use lowercase letters, numbers, and hyphens (must start and end with a letter or number).",
      );
      return;
    }
    setValidationError(null);
    setNewProfileName(name);
    setNewProfileDisplayName(name);
    setMode("create-display");
  }, []);

  // 新規プロファイル作成完了
  const handleCreateProfileSubmit = useCallback(
    async (displayName: string) => {
      try {
        await createProfile(newProfileName, {
          displayName,
          env: {},
        });
        setSelectedProfileName(newProfileName);
        setFocus("env"); // viewモードでは環境変数にフォーカス
        setMode("view");
      } catch {
        // エラー状態は useProfiles フック側で管理するため、ここでは一覧に戻す
        setMode("list");
      }
    },
    [createProfile, newProfileName],
  );

  // プロファイル削除確認
  const handleConfirmDeleteProfile = useCallback(
    async (confirmed: boolean) => {
      if (confirmed && selectedProfileName) {
        try {
          await deleteProfile(selectedProfileName);
          setSelectedProfileName(null);
        } catch {
          // エラー状態は useProfiles フック側で管理するため、ここでは握りつぶす
        }
      }
      setMode("list");
    },
    [deleteProfile, selectedProfileName],
  );

  // 環境変数追加開始
  const handleStartAddEnv = useCallback(() => {
    setNewEnvKey("");
    setNewEnvValue("");
    setValidationError(null);
    setMode("add-env-key");
  }, []);

  // 環境変数キー入力完了
  const handleEnvKeySubmit = useCallback((key: string) => {
    const trimmedKey = key.trim();
    if (!trimmedKey || !ENV_VAR_KEY_PATTERN.test(trimmedKey)) {
      setValidationError(
        "Invalid variable name. Use letters, numbers, and underscores (must start with a letter or underscore).",
      );
      setNewEnvKey(trimmedKey);
      return;
    }

    setValidationError(null);
    setNewEnvKey(trimmedKey);
    setMode("add-env-value");
  }, []);

  // 環境変数追加完了
  const handleEnvValueSubmit = useCallback(
    async (value: string) => {
      if (selectedProfileName) {
        try {
          await updateEnvVar(selectedProfileName, newEnvKey, value);
        } catch {
          // エラー状態は useProfiles フック側で管理するため、ここでは握りつぶす
        }
      }
      setMode("view");
    },
    [updateEnvVar, selectedProfileName, newEnvKey],
  );

  // 環境変数編集開始
  const handleStartEditEnv = useCallback(
    (key: string, currentValue: string) => {
      setSelectedEnvKey(key);
      setNewEnvValue(currentValue);
      setMode("edit-env-value");
    },
    [],
  );

  // 環境変数編集完了
  const handleEditEnvSubmit = useCallback(
    async (value: string) => {
      if (selectedProfileName && selectedEnvKey) {
        try {
          await updateEnvVar(selectedProfileName, selectedEnvKey, value);
        } catch {
          // エラー状態は useProfiles フック側で管理するため、ここでは握りつぶす
        }
      }
      setMode("view");
    },
    [updateEnvVar, selectedProfileName, selectedEnvKey],
  );

  // 環境変数削除確認
  const handleConfirmDeleteEnv = useCallback(
    async (confirmed: boolean) => {
      if (confirmed && selectedProfileName && selectedEnvKey) {
        try {
          await deleteEnvVar(selectedProfileName, selectedEnvKey);
        } catch {
          // エラー状態は useProfiles フック側で管理するため、ここでは握りつぶす
        }
      }
      setMode("view");
    },
    [deleteEnvVar, selectedProfileName, selectedEnvKey],
  );

  // キーボード入力ハンドリング
  useAppInput(
    (input, key) => {
      // 入力モード時は他のキーハンドリングをスキップ
      if (
        mode === "create-name" ||
        mode === "create-display" ||
        mode === "add-env-key" ||
        mode === "add-env-value" ||
        mode === "edit-env-value"
      ) {
        if (key.escape) {
          setMode(mode.startsWith("create") ? "list" : "view");
        }
        return;
      }

      // 確認ダイアログ時は Confirm コンポーネントがハンドリング
      if (mode === "confirm-delete-profile" || mode === "confirm-delete-env") {
        return;
      }

      // Escape で戻る
      if (key.escape) {
        if (mode === "view") {
          setMode("list");
          setSelectedProfileName(null);
        } else {
          onBack();
        }
        return;
      }

      // プロファイル一覧モード
      if (mode === "list") {
        if (input === "n") {
          handleStartCreateProfile();
          return;
        }
        if (input === "d" && profileItems.length > 0) {
          const item = profileItems[profileIndex];
          if (item?.profileName && item.profileName !== activeProfileName) {
            setSelectedProfileName(item.profileName);
            setMode("confirm-delete-profile");
          }
          return;
        }
        return;
      }

      // プロファイル詳細モード
      if (mode === "view") {
        // Tab でフォーカス切り替え (env ↔ osenv)
        if (key.tab) {
          setFocus((prev) => (prev === "env" ? "osenv" : "env"));
          return;
        }

        // j/k でスクロール
        if (input === "j" || key.downArrow) {
          if (focus === "env") {
            setEnvIndex((prev) =>
              Math.min(prev + 1, Math.max(0, envItems.length - 1)),
            );
          } else if (focus === "osenv") {
            setOsEnvIndex((prev) =>
              Math.min(prev + 1, Math.max(0, osEnvItems.length - 1)),
            );
          }
          return;
        }
        if (input === "k" || key.upArrow) {
          if (focus === "env") {
            setEnvIndex((prev) => Math.max(prev - 1, 0));
          } else if (focus === "osenv") {
            setOsEnvIndex((prev) => Math.max(prev - 1, 0));
          }
          return;
        }

        // 環境変数操作
        if (focus === "env") {
          if (input === "a") {
            handleStartAddEnv();
            return;
          }
          if (input === "e" && envItems.length > 0) {
            const item = envItems[envIndex];
            if (item) {
              handleStartEditEnv(item.key, item.envValue);
            }
            return;
          }
          if (input === "d" && envItems.length > 0) {
            const item = envItems[envIndex];
            if (item) {
              setSelectedEnvKey(item.key);
              setMode("confirm-delete-env");
            }
            return;
          }
        }
      }
    },
    { isActive: true },
  );

  // フッターアクション
  const getFooterActions = () => {
    if (mode === "list") {
      return [
        { key: "enter", description: "Select" },
        { key: "n", description: "New" },
        { key: "d", description: "Delete" },
        { key: "esc", description: "Back" },
      ];
    }
    if (mode === "view") {
      return [
        { key: "tab", description: "Switch focus" },
        { key: "j/k", description: "Scroll" },
        { key: "a", description: "Add env" },
        { key: "e", description: "Edit env" },
        { key: "d", description: "Delete env" },
        { key: "esc", description: "Back" },
      ];
    }
    return [{ key: "esc", description: "Cancel" }];
  };

  // ローディング表示
  if (loading) {
    return (
      <Box flexDirection="column" height={rows}>
        <Header
          title="Environment Profiles"
          titleColor="magenta"
          version={version}
        />
        <Box flexGrow={1} justifyContent="center" alignItems="center">
          <Text>Loading profiles...</Text>
        </Box>
      </Box>
    );
  }

  // エラー表示
  if (error) {
    return (
      <Box flexDirection="column" height={rows}>
        <Header
          title="Environment Profiles"
          titleColor="magenta"
          version={version}
        />
        <Box flexGrow={1} marginTop={1}>
          <Text color="red">Error: {error.message}</Text>
        </Box>
        <Footer actions={[{ key: "esc", description: "Back" }]} />
      </Box>
    );
  }

  // 新規プロファイル作成：名前入力
  if (mode === "create-name") {
    return (
      <Box flexDirection="column" height={rows}>
        <Header
          title="Environment Profiles"
          titleColor="magenta"
          version={version}
        />
        <Box flexDirection="column" flexGrow={1} marginTop={1}>
          <Text>Create new profile</Text>
          <Box marginTop={1}>
            <Input
              value={newProfileName}
              onChange={setNewProfileName}
              onSubmit={handleCreateNameSubmit}
              label="Profile name (lowercase, a-z0-9-):"
              placeholder="development"
            />
          </Box>
          {validationError && (
            <Box marginTop={1}>
              <Text color="red">{validationError}</Text>
            </Box>
          )}
        </Box>
        <Footer actions={getFooterActions()} />
      </Box>
    );
  }

  // 新規プロファイル作成：表示名入力
  if (mode === "create-display") {
    return (
      <Box flexDirection="column" height={rows}>
        <Header
          title="Environment Profiles"
          titleColor="magenta"
          version={version}
        />
        <Box flexDirection="column" flexGrow={1} marginTop={1}>
          <Text>Create new profile: {newProfileName}</Text>
          <Box marginTop={1}>
            <Input
              value={newProfileDisplayName}
              onChange={setNewProfileDisplayName}
              onSubmit={handleCreateProfileSubmit}
              label="Display name:"
              placeholder="Development"
            />
          </Box>
        </Box>
        <Footer actions={getFooterActions()} />
      </Box>
    );
  }

  // 環境変数追加：キー入力
  if (mode === "add-env-key") {
    return (
      <Box flexDirection="column" height={rows}>
        <Header
          title="Environment Profiles"
          titleColor="magenta"
          version={version}
        />
        <Box flexDirection="column" flexGrow={1} marginTop={1}>
          <Text>Add environment variable</Text>
          <Box marginTop={1}>
            <Input
              value={newEnvKey}
              onChange={setNewEnvKey}
              onSubmit={handleEnvKeySubmit}
              label="Variable name:"
              placeholder="API_KEY"
            />
          </Box>
          {validationError && (
            <Box marginTop={1}>
              <Text color="red">{validationError}</Text>
            </Box>
          )}
        </Box>
        <Footer actions={getFooterActions()} />
      </Box>
    );
  }

  // 環境変数追加：値入力
  if (mode === "add-env-value") {
    return (
      <Box flexDirection="column" height={rows}>
        <Header
          title="Environment Profiles"
          titleColor="magenta"
          version={version}
        />
        <Box flexDirection="column" flexGrow={1} marginTop={1}>
          <Text>Add environment variable: {newEnvKey}</Text>
          <Box marginTop={1}>
            <Input
              value={newEnvValue}
              onChange={setNewEnvValue}
              onSubmit={handleEnvValueSubmit}
              label="Value:"
              placeholder="your-value"
            />
          </Box>
        </Box>
        <Footer actions={getFooterActions()} />
      </Box>
    );
  }

  // 環境変数編集：値入力
  if (mode === "edit-env-value") {
    return (
      <Box flexDirection="column" height={rows}>
        <Header
          title="Environment Profiles"
          titleColor="magenta"
          version={version}
        />
        <Box flexDirection="column" flexGrow={1} marginTop={1}>
          <Text>Edit environment variable: {selectedEnvKey}</Text>
          <Box marginTop={1}>
            <Input
              value={newEnvValue}
              onChange={setNewEnvValue}
              onSubmit={handleEditEnvSubmit}
              label="New value:"
              placeholder="new-value"
            />
          </Box>
        </Box>
        <Footer actions={getFooterActions()} />
      </Box>
    );
  }

  // プロファイル削除確認
  if (mode === "confirm-delete-profile") {
    const profileToDelete = selectedProfileName
      ? profiles?.profiles[selectedProfileName]
      : null;
    return (
      <Box flexDirection="column" height={rows}>
        <Header
          title="Environment Profiles"
          titleColor="magenta"
          version={version}
        />
        <Box flexDirection="column" flexGrow={1} marginTop={1}>
          <Confirm
            message={`Delete profile "${profileToDelete?.displayName ?? selectedProfileName}"?`}
            onConfirm={handleConfirmDeleteProfile}
          />
        </Box>
      </Box>
    );
  }

  // 環境変数削除確認
  if (mode === "confirm-delete-env") {
    return (
      <Box flexDirection="column" height={rows}>
        <Header
          title="Environment Profiles"
          titleColor="magenta"
          version={version}
        />
        <Box flexDirection="column" flexGrow={1} marginTop={1}>
          <Confirm
            message={`Delete environment variable "${selectedEnvKey}"?`}
            onConfirm={handleConfirmDeleteEnv}
          />
        </Box>
      </Box>
    );
  }

  // プロファイル一覧
  if (mode === "list") {
    const hasProfiles = Boolean(
      profiles && Object.keys(profiles.profiles).length > 0,
    );
    return (
      <Box flexDirection="column" height={rows}>
        <Header
          title="Environment Profiles"
          titleColor="magenta"
          version={version}
          activeProfile={activeProfileName}
        />

        <Box flexDirection="column" flexGrow={1} marginTop={1}>
          <Box marginBottom={1}>
            <Text>Select a profile to activate (or choose "(none)"):</Text>
          </Box>

          <Select
            items={profileItems}
            onSelect={handleActivateProfile}
            selectedIndex={profileIndex}
            onSelectedIndexChange={setProfileIndex}
          />
          {!hasProfiles && (
            <Box marginTop={1}>
              <Text dimColor>No profiles. Press [n] to create one.</Text>
            </Box>
          )}
        </Box>

        <Footer actions={getFooterActions()} />
      </Box>
    );
  }

  // プロファイル詳細
  const maxOsEnvVisible = Math.max(
    5,
    Math.floor((rows - UI_CHROME_HEIGHT) / 2),
  );

  return (
    <Box flexDirection="column" height={rows}>
      <Header
        title="Environment Profiles"
        titleColor="magenta"
        version={version}
        activeProfile={activeProfileName}
      />

      <Box flexDirection="column" flexGrow={1} marginTop={1}>
        {/* プロファイル情報 */}
        <Box marginBottom={1}>
          <Text bold>Profile: </Text>
          <Text color="cyan">
            {currentProfile?.displayName ?? selectedProfileName}
          </Text>
          {selectedProfileName === activeProfileName && (
            <Text color="green"> (active)</Text>
          )}
        </Box>

        {currentProfile?.description && (
          <Box marginBottom={1}>
            <Text dimColor>{currentProfile.description}</Text>
          </Box>
        )}

        {/* プロファイル環境変数 */}
        <Box marginBottom={1}>
          <Text bold {...(focus === "env" ? { color: "cyan" as const } : {})}>
            Profile Environment Variables:
          </Text>
        </Box>

        <Box flexDirection="column" marginLeft={2} marginBottom={1}>
          {envItems.length === 0 ? (
            <Text dimColor>No environment variables. Press [a] to add.</Text>
          ) : (
            envItems.map((item, idx) => {
              const isEnvSelected = focus === "env" && idx === envIndex;
              return (
                <Box key={item.key}>
                  <Text
                    {...(isEnvSelected ? { color: "cyan" as const } : {})}
                    inverse={isEnvSelected}
                  >
                    {item.key}
                  </Text>
                  <Text>=</Text>
                  <Text>{item.envValue}</Text>
                </Box>
              );
            })
          )}
        </Box>

        {/* OS環境変数（上書きされるものをハイライト） */}
        <Box marginBottom={1}>
          <Text bold {...(focus === "osenv" ? { color: "cyan" as const } : {})}>
            OS Environment (overwritten keys in yellow):
          </Text>
        </Box>

        <Box flexDirection="column" marginLeft={2} overflow="hidden">
          {osEnvItems
            .slice(osEnvIndex, osEnvIndex + maxOsEnvVisible)
            .map((item, idx) => {
              // osEnvIndex は「選択中のOS環境変数のインデックス」であり、同時にスクロールの先頭位置でもある
              // そのため、表示上は slice した先頭要素が選択状態になる
              const actualIndex = osEnvIndex + idx;
              const isOsEnvSelected =
                focus === "osenv" && actualIndex === osEnvIndex;
              return (
                <Box key={item.key}>
                  <Text
                    {...(item.isOverwritten
                      ? { color: "yellow" as const }
                      : {})}
                    inverse={isOsEnvSelected}
                  >
                    {item.key}
                  </Text>
                  <Text>=</Text>
                  <Text dimColor>
                    {item.value.slice(0, 50)}
                    {item.value.length > 50 ? "..." : ""}
                  </Text>
                </Box>
              );
            })}
          {osEnvItems.length > maxOsEnvVisible && (
            <Text dimColor>
              ... ({osEnvIndex + 1}-
              {Math.min(osEnvIndex + maxOsEnvVisible, osEnvItems.length)} of{" "}
              {osEnvItems.length})
            </Text>
          )}
        </Box>
      </Box>

      <Footer actions={getFooterActions()} />
    </Box>
  );
}
