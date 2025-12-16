/**
 * プロファイル管理フック
 *
 * 環境変数プロファイルの読み込み・操作を提供します。
 * @see specs/SPEC-dafff079/spec.md
 */

import { useState, useEffect, useCallback } from "react";
import {
  loadProfiles,
  setActiveProfile as setActiveProfileApi,
  createProfile as createProfileApi,
  updateProfile as updateProfileApi,
  deleteProfile as deleteProfileApi,
} from "../../../config/profiles.js";
import type {
  ProfilesConfig,
  EnvironmentProfile,
} from "../../../types/profiles.js";

export interface UseProfilesResult {
  /** プロファイル設定（ロード中はnull） */
  profiles: ProfilesConfig | null;
  /** ロード中フラグ */
  loading: boolean;
  /** エラー（なければnull） */
  error: Error | null;
  /** アクティブなプロファイル名 */
  activeProfileName: string | null;
  /** アクティブなプロファイル設定 */
  activeProfile: EnvironmentProfile | null;

  /** プロファイル設定を再読み込み */
  refresh: () => Promise<void>;
  /** アクティブなプロファイルを設定 */
  setActiveProfile: (name: string | null) => Promise<void>;
  /** 新しいプロファイルを作成 */
  createProfile: (name: string, profile: EnvironmentProfile) => Promise<void>;
  /** プロファイルを更新 */
  updateProfile: (
    name: string,
    updates: Partial<EnvironmentProfile>,
  ) => Promise<void>;
  /** プロファイルを削除 */
  deleteProfile: (name: string) => Promise<void>;
  /** 環境変数を更新 */
  updateEnvVar: (
    profileName: string,
    key: string,
    value: string,
  ) => Promise<void>;
  /** 環境変数を削除 */
  deleteEnvVar: (profileName: string, key: string) => Promise<void>;
}

/**
 * プロファイル管理フック
 *
 * コンポーネントでプロファイルの読み込みと操作を行うためのフック。
 * 初回マウント時に自動的にプロファイル設定を読み込みます。
 */
export function useProfiles(): UseProfilesResult {
  const [profiles, setProfiles] = useState<ProfilesConfig | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<Error | null>(null);

  // プロファイル設定を読み込む
  const refresh = useCallback(async () => {
    try {
      setLoading(true);
      setError(null);
      const config = await loadProfiles();
      setProfiles(config);
    } catch (err) {
      setError(err instanceof Error ? err : new Error(String(err)));
    } finally {
      setLoading(false);
    }
  }, []);

  // 初回マウント時に読み込み
  useEffect(() => {
    refresh();
  }, [refresh]);

  // アクティブなプロファイル名
  const activeProfileName = profiles?.activeProfile ?? null;

  // アクティブなプロファイル設定
  const activeProfile = activeProfileName
    ? (profiles?.profiles[activeProfileName] ?? null)
    : null;

  // アクティブなプロファイルを設定
  const setActiveProfile = useCallback(
    async (name: string | null) => {
      try {
        await setActiveProfileApi(name);
        await refresh();
      } catch (err) {
        setError(err instanceof Error ? err : new Error(String(err)));
        throw err;
      }
    },
    [refresh],
  );

  // プロファイルを作成
  const createProfile = useCallback(
    async (name: string, profile: EnvironmentProfile) => {
      try {
        await createProfileApi(name, profile);
        await refresh();
      } catch (err) {
        setError(err instanceof Error ? err : new Error(String(err)));
        throw err;
      }
    },
    [refresh],
  );

  // プロファイルを更新
  const updateProfile = useCallback(
    async (name: string, updates: Partial<EnvironmentProfile>) => {
      try {
        await updateProfileApi(name, updates);
        await refresh();
      } catch (err) {
        setError(err instanceof Error ? err : new Error(String(err)));
        throw err;
      }
    },
    [refresh],
  );

  // プロファイルを削除
  const deleteProfile = useCallback(
    async (name: string) => {
      try {
        await deleteProfileApi(name);
        await refresh();
      } catch (err) {
        setError(err instanceof Error ? err : new Error(String(err)));
        throw err;
      }
    },
    [refresh],
  );

  // 環境変数を更新
  const updateEnvVar = useCallback(
    async (profileName: string, key: string, value: string) => {
      if (!profiles) {
        const err = new Error("Profiles not loaded");
        setError(err);
        throw err;
      }

      if (!profiles.profiles[profileName]) {
        const err = new Error(`Profile "${profileName}" does not exist`);
        setError(err);
        throw err;
      }

      const existingProfile = profiles.profiles[profileName];
      const newEnv = { ...existingProfile.env, [key]: value };

      await updateProfile(profileName, { env: newEnv });
    },
    [profiles, updateProfile],
  );

  // 環境変数を削除
  const deleteEnvVar = useCallback(
    async (profileName: string, key: string) => {
      if (!profiles) {
        const err = new Error("Profiles not loaded");
        setError(err);
        throw err;
      }

      if (!profiles.profiles[profileName]) {
        const err = new Error(`Profile "${profileName}" does not exist`);
        setError(err);
        throw err;
      }

      const existingProfile = profiles.profiles[profileName];
      const newEnv = { ...existingProfile.env };
      delete newEnv[key];

      await updateProfile(profileName, { env: newEnv });
    },
    [profiles, updateProfile],
  );

  return {
    profiles,
    loading,
    error,
    activeProfileName,
    activeProfile,
    refresh,
    setActiveProfile,
    createProfile,
    updateProfile,
    deleteProfile,
    updateEnvVar,
    deleteEnvVar,
  };
}
