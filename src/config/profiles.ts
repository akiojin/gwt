/**
 * 環境変数プロファイル設定管理
 *
 * ~/.gwt/profiles.yamlからプロファイル設定を読み込み、
 * AIツール起動時の環境変数を管理します。
 *
 * @see specs/SPEC-dafff079/spec.md
 */

import { mkdir, readFile, rename, writeFile } from "node:fs/promises";
import path from "node:path";
import { parse as parseYaml, stringify as stringifyYaml } from "yaml";
import { homedir } from "node:os";
import {
  DEFAULT_PROFILES_CONFIG,
  isValidProfileName,
  type EnvironmentProfile,
  type ProfilesConfig,
} from "../types/profiles.js";

/**
 * 設定ディレクトリのパスを取得
 *
 * 環境変数の優先順位: GWT_HOME > CLAUDE_WORKTREE_HOME (後方互換性) > ホームディレクトリ
 */
function getConfigDir(): string {
  const worktreeHome =
    process.env.GWT_HOME && process.env.GWT_HOME.trim().length > 0
      ? process.env.GWT_HOME
      : process.env.CLAUDE_WORKTREE_HOME &&
          process.env.CLAUDE_WORKTREE_HOME.trim().length > 0
        ? process.env.CLAUDE_WORKTREE_HOME
        : homedir();
  return path.join(worktreeHome, ".gwt");
}

/**
 * プロファイル設定ファイルのパスを取得
 */
function getProfilesConfigPath(): string {
  return path.join(getConfigDir(), "profiles.yaml");
}

/**
 * プロファイル設定ファイルのパス（後方互換性のためエクスポート）
 *
 * @deprecated 内部では getProfilesConfigPath() を使用してください。
 * この定数はモジュールロード時に評価されるため、
 * 環境変数の空文字チェック（trim().length > 0）が行われません。
 */
export const PROFILES_CONFIG_PATH = path.join(
  process.env.GWT_HOME || process.env.CLAUDE_WORKTREE_HOME || homedir(),
  ".gwt",
  "profiles.yaml",
);

/**
 * プロファイル設定を読み込む
 *
 * ~/.gwt/profiles.yamlから設定を読み込みます。
 * ファイルが存在しない場合はデフォルト設定を返します。
 *
 * @returns ProfilesConfig
 * @throws YAML構文エラー時
 */
export async function loadProfiles(): Promise<ProfilesConfig> {
  try {
    const configPath = getProfilesConfigPath();
    const content = await readFile(configPath, "utf-8");
    const config = parseYaml(content) as ProfilesConfig;

    // 基本的な検証
    if (!config.version || typeof config.version !== "string") {
      throw new Error("version field is required and must be a string");
    }

    if (config.profiles && typeof config.profiles !== "object") {
      throw new Error("profiles field must be an object");
    }

    return {
      version: config.version,
      activeProfile: config.activeProfile ?? null,
      profiles: config.profiles ?? {},
    };
  } catch (error) {
    // ファイルが存在しない場合はデフォルト設定を返す
    if (error instanceof Error && "code" in error && error.code === "ENOENT") {
      // DEFAULT_PROFILES_CONFIG は不変オブジェクトのため、
      // 呼び出し側が編集できるように新しい参照を返す
      return {
        version: DEFAULT_PROFILES_CONFIG.version,
        activeProfile: null,
        profiles: {},
      };
    }

    // YAML構文エラーの場合
    throw error;
  }
}

/**
 * プロファイル設定を保存する
 *
 * ~/.gwt/profiles.yamlに設定を保存します。
 * ディレクトリが存在しない場合は自動的に作成します。
 *
 * @param config - 保存するプロファイル設定
 */
export async function saveProfiles(config: ProfilesConfig): Promise<void> {
  const configDir = getConfigDir();
  const configPath = getProfilesConfigPath();
  const tempPath = `${configPath}.tmp`;

  await mkdir(configDir, { recursive: true });

  const yaml = stringifyYaml(config);
  await writeFile(tempPath, yaml, { mode: 0o600 });
  await rename(tempPath, configPath);
}

/**
 * アクティブなプロファイルを取得
 *
 * 現在選択されているプロファイルの設定を返します。
 * プロファイルが選択されていない場合、または選択されたプロファイルが
 * 存在しない場合はnullを返します。
 *
 * @returns アクティブなプロファイル、またはnull
 */
export async function getActiveProfile(): Promise<EnvironmentProfile | null> {
  const config = await loadProfiles();

  if (!config.activeProfile) {
    return null;
  }

  const profile = config.profiles[config.activeProfile];
  return profile ?? null;
}

/**
 * アクティブなプロファイル名を取得
 *
 * @returns アクティブなプロファイル名、またはnull
 */
export async function getActiveProfileName(): Promise<string | null> {
  const config = await loadProfiles();
  return config.activeProfile;
}

/**
 * アクティブなプロファイルを設定
 *
 * @param profileName - 設定するプロファイル名（nullで無効化）
 * @throws プロファイルが存在しない場合
 */
export async function setActiveProfile(
  profileName: string | null,
): Promise<void> {
  const config = await loadProfiles();

  if (profileName !== null && !config.profiles[profileName]) {
    throw new Error(`Profile "${profileName}" does not exist`);
  }

  config.activeProfile = profileName;
  await saveProfiles(config);
}

/**
 * 新しいプロファイルを作成
 *
 * @param name - プロファイル名
 * @param profile - プロファイル設定
 * @throws 既存のプロファイル名の場合
 * @throws 無効なプロファイル名の場合
 */
export async function createProfile(
  name: string,
  profile: EnvironmentProfile,
): Promise<void> {
  if (!isValidProfileName(name)) {
    throw new Error(
      `Invalid profile name: "${name}". Use only lowercase letters, numbers, and hyphens.`,
    );
  }

  const config = await loadProfiles();

  if (config.profiles[name]) {
    throw new Error(`Profile "${name}" already exists`);
  }

  config.profiles[name] = profile;
  await saveProfiles(config);
}

/**
 * プロファイルを更新
 *
 * @param name - プロファイル名
 * @param updates - 更新するフィールド（envが指定された場合は完全に置き換えられます）
 * @throws プロファイルが存在しない場合
 */
export async function updateProfile(
  name: string,
  updates: Partial<EnvironmentProfile>,
): Promise<void> {
  const config = await loadProfiles();

  if (!config.profiles[name]) {
    throw new Error(`Profile "${name}" does not exist`);
  }

  config.profiles[name] = {
    ...config.profiles[name],
    ...updates,
  };
  await saveProfiles(config);
}

/**
 * プロファイルを削除
 *
 * @param name - プロファイル名
 * @throws アクティブなプロファイルの場合
 * @throws プロファイルが存在しない場合
 */
export async function deleteProfile(name: string): Promise<void> {
  const config = await loadProfiles();

  if (!config.profiles[name]) {
    throw new Error(`Profile "${name}" does not exist`);
  }

  if (config.activeProfile === name) {
    throw new Error(
      `Cannot delete active profile "${name}". Please switch to another profile first.`,
    );
  }

  delete config.profiles[name];
  await saveProfiles(config);
}

/**
 * アクティブなプロファイルの環境変数を解決
 *
 * AIツール起動時に使用する環境変数を返します。
 * プロファイルが選択されていない場合は空オブジェクトを返します。
 *
 * @returns 環境変数のRecord
 */
export async function resolveProfileEnv(): Promise<Record<string, string>> {
  const profile = await getActiveProfile();

  if (!profile) {
    return {};
  }

  return { ...profile.env };
}
