/**
 * 環境変数プロファイル管理機能のテスト
 * @see specs/SPEC-dafff079/spec.md
 */

import { describe, it, expect, beforeEach, afterEach, mock } from "bun:test";
import { mkdir, rm, writeFile } from "node:fs/promises";
import path from "node:path";
import { tmpdir } from "node:os";

import {
  isValidProfileName,
  DEFAULT_PROFILES_CONFIG,
  type ProfilesConfig,
  type EnvironmentProfile,
} from "../../../src/types/profiles.js";

// 動的インポート用の変数
let loadProfiles: () => Promise<ProfilesConfig>;
let saveProfiles: (config: ProfilesConfig) => Promise<void>;
let getActiveProfile: () => Promise<EnvironmentProfile | null>;
let getActiveProfileName: () => Promise<string | null>;
let setActiveProfile: (name: string | null) => Promise<void>;
let createProfile: (name: string, profile: EnvironmentProfile) => Promise<void>;
let updateProfile: (
  name: string,
  updates: Partial<EnvironmentProfile>,
) => Promise<void>;
let deleteProfile: (name: string) => Promise<void>;
let resolveProfileEnv: () => Promise<Record<string, string>>;

const originalGwtHome = process.env.GWT_HOME;

describe("isValidProfileName", () => {
  it("英数字とハイフンのみの名前は有効", () => {
    expect(isValidProfileName("development")).toBe(true);
    expect(isValidProfileName("prod-env")).toBe(true);
    expect(isValidProfileName("test-123")).toBe(true);
  });

  it("空文字は無効", () => {
    expect(isValidProfileName("")).toBe(false);
  });

  it("大文字を含む名前は無効", () => {
    expect(isValidProfileName("Development")).toBe(false);
    expect(isValidProfileName("PROD")).toBe(false);
  });

  it("特殊文字を含む名前は無効", () => {
    expect(isValidProfileName("dev_env")).toBe(false);
    expect(isValidProfileName("dev.env")).toBe(false);
    expect(isValidProfileName("dev env")).toBe(false);
    expect(isValidProfileName("dev/env")).toBe(false);
  });

  it("先頭/末尾がハイフン、またはハイフンのみの名前は無効", () => {
    expect(isValidProfileName("-")).toBe(false);
    expect(isValidProfileName("---")).toBe(false);
    expect(isValidProfileName("-dev")).toBe(false);
    expect(isValidProfileName("dev-")).toBe(false);
    expect(isValidProfileName("-dev-")).toBe(false);
  });
});

describe("loadProfiles", () => {
  const testDir = path.join(tmpdir(), "gwt-test-profiles");

  beforeEach(async () => {
    process.env.GWT_HOME = testDir;
    await mkdir(path.join(testDir, ".gwt"), { recursive: true });
    // resetModules not needed in bun;
    const module = await import("../../../src/config/profiles.js");
    loadProfiles = module.loadProfiles;
    saveProfiles = module.saveProfiles;
  });

  afterEach(async () => {
    if (originalGwtHome === undefined) {
      delete process.env.GWT_HOME;
    } else {
      process.env.GWT_HOME = originalGwtHome;
    }
    await rm(testDir, { recursive: true, force: true });
    // resetModules not needed in bun;
  });

  it("設定ファイルが存在しない場合、デフォルト設定を返す", async () => {
    const config = await loadProfiles();
    expect(config).toEqual(DEFAULT_PROFILES_CONFIG);
  });

  it("有効なYAMLファイルを正常に読み込める", async () => {
    const yamlContent = `
version: "1.0"
activeProfile: "development"
profiles:
  development:
    displayName: "Development"
    description: "開発環境用"
    env:
      DEBUG: "true"
      API_KEY: "dev-key"
`;
    await writeFile(
      path.join(testDir, ".gwt", "profiles.yaml"),
      yamlContent,
      "utf-8",
    );

    const config = await loadProfiles();
    expect(config.version).toBe("1.0");
    expect(config.activeProfile).toBe("development");
    expect(config.profiles.development.displayName).toBe("Development");
    expect(config.profiles.development.env.DEBUG).toBe("true");
  });

  it("不正なYAML形式の場合、エラーをスローする", async () => {
    const invalidYaml = `
version: "1.0"
activeProfile: "development
profiles: {invalid
`;
    await writeFile(
      path.join(testDir, ".gwt", "profiles.yaml"),
      invalidYaml,
      "utf-8",
    );

    await expect(loadProfiles()).rejects.toThrow(/line|column/i);
  });
});

describe("saveProfiles", () => {
  const testDir = path.join(tmpdir(), "gwt-test-profiles-save");

  beforeEach(async () => {
    process.env.GWT_HOME = testDir;
    await mkdir(path.join(testDir, ".gwt"), { recursive: true });
    // resetModules not needed in bun;
    const module = await import("../../../src/config/profiles.js");
    loadProfiles = module.loadProfiles;
    saveProfiles = module.saveProfiles;
  });

  afterEach(async () => {
    if (originalGwtHome === undefined) {
      delete process.env.GWT_HOME;
    } else {
      process.env.GWT_HOME = originalGwtHome;
    }
    await rm(testDir, { recursive: true, force: true });
    // resetModules not needed in bun;
  });

  it("設定を正常に保存できる", async () => {
    const config: ProfilesConfig = {
      version: "1.0",
      activeProfile: "production",
      profiles: {
        production: {
          displayName: "Production",
          env: { NODE_ENV: "production" },
        },
      },
    };

    await saveProfiles(config);
    const loaded = await loadProfiles();

    expect(loaded.activeProfile).toBe("production");
    expect(loaded.profiles.production.env.NODE_ENV).toBe("production");
  });

  it("ディレクトリが存在しない場合、自動的に作成する", async () => {
    await rm(testDir, { recursive: true, force: true });

    const config: ProfilesConfig = {
      version: "1.0",
      activeProfile: null,
      profiles: {},
    };

    await saveProfiles(config);
    const loaded = await loadProfiles();

    expect(loaded).toEqual(config);
  });
});

describe("getActiveProfile", () => {
  const testDir = path.join(tmpdir(), "gwt-test-profiles-active");

  beforeEach(async () => {
    process.env.GWT_HOME = testDir;
    await mkdir(path.join(testDir, ".gwt"), { recursive: true });
    // resetModules not needed in bun;
    const module = await import("../../../src/config/profiles.js");
    loadProfiles = module.loadProfiles;
    saveProfiles = module.saveProfiles;
    getActiveProfile = module.getActiveProfile;
  });

  afterEach(async () => {
    if (originalGwtHome === undefined) {
      delete process.env.GWT_HOME;
    } else {
      process.env.GWT_HOME = originalGwtHome;
    }
    await rm(testDir, { recursive: true, force: true });
    // resetModules not needed in bun;
  });

  it("アクティブなプロファイルが設定されている場合、そのプロファイルを返す", async () => {
    const yamlContent = `
version: "1.0"
activeProfile: "development"
profiles:
  development:
    displayName: "Development"
    env:
      DEBUG: "true"
`;
    await writeFile(
      path.join(testDir, ".gwt", "profiles.yaml"),
      yamlContent,
      "utf-8",
    );

    const profile = await getActiveProfile();
    expect(profile).not.toBeNull();
    expect(profile?.displayName).toBe("Development");
    expect(profile?.env.DEBUG).toBe("true");
  });

  it("アクティブなプロファイルが設定されていない場合、nullを返す", async () => {
    const yamlContent = `
version: "1.0"
activeProfile: null
profiles: {}
`;
    await writeFile(
      path.join(testDir, ".gwt", "profiles.yaml"),
      yamlContent,
      "utf-8",
    );

    const profile = await getActiveProfile();
    expect(profile).toBeNull();
  });

  it("アクティブなプロファイルが存在しない場合、nullを返す", async () => {
    const yamlContent = `
version: "1.0"
activeProfile: "nonexistent"
profiles:
  development:
    displayName: "Development"
    env: {}
`;
    await writeFile(
      path.join(testDir, ".gwt", "profiles.yaml"),
      yamlContent,
      "utf-8",
    );

    const profile = await getActiveProfile();
    expect(profile).toBeNull();
  });
});

describe("getActiveProfileName", () => {
  const testDir = path.join(tmpdir(), "gwt-test-profiles-name");

  beforeEach(async () => {
    process.env.GWT_HOME = testDir;
    await mkdir(path.join(testDir, ".gwt"), { recursive: true });
    // resetModules not needed in bun;
    const module = await import("../../../src/config/profiles.js");
    getActiveProfileName = module.getActiveProfileName;
  });

  afterEach(async () => {
    if (originalGwtHome === undefined) {
      delete process.env.GWT_HOME;
    } else {
      process.env.GWT_HOME = originalGwtHome;
    }
    await rm(testDir, { recursive: true, force: true });
    // resetModules not needed in bun;
  });

  it("アクティブなプロファイル名を返す", async () => {
    const yamlContent = `
version: "1.0"
activeProfile: "development"
profiles:
  development:
    displayName: "Development"
    env: {}
`;
    await writeFile(
      path.join(testDir, ".gwt", "profiles.yaml"),
      yamlContent,
      "utf-8",
    );

    const name = await getActiveProfileName();
    expect(name).toBe("development");
  });

  it("アクティブなプロファイルがない場合、nullを返す", async () => {
    const yamlContent = `
version: "1.0"
activeProfile: null
profiles: {}
`;
    await writeFile(
      path.join(testDir, ".gwt", "profiles.yaml"),
      yamlContent,
      "utf-8",
    );

    const name = await getActiveProfileName();
    expect(name).toBeNull();
  });
});

describe("setActiveProfile", () => {
  const testDir = path.join(tmpdir(), "gwt-test-profiles-set");

  beforeEach(async () => {
    process.env.GWT_HOME = testDir;
    await mkdir(path.join(testDir, ".gwt"), { recursive: true });
    // resetModules not needed in bun;
    const module = await import("../../../src/config/profiles.js");
    setActiveProfile = module.setActiveProfile;
    getActiveProfileName = module.getActiveProfileName;
  });

  afterEach(async () => {
    if (originalGwtHome === undefined) {
      delete process.env.GWT_HOME;
    } else {
      process.env.GWT_HOME = originalGwtHome;
    }
    await rm(testDir, { recursive: true, force: true });
    // resetModules not needed in bun;
  });

  it("アクティブなプロファイルを設定できる", async () => {
    const yamlContent = `
version: "1.0"
activeProfile: null
profiles:
  development:
    displayName: "Development"
    env: {}
  production:
    displayName: "Production"
    env: {}
`;
    await writeFile(
      path.join(testDir, ".gwt", "profiles.yaml"),
      yamlContent,
      "utf-8",
    );

    await setActiveProfile("production");

    const name = await getActiveProfileName();
    expect(name).toBe("production");
  });

  it("nullを設定してプロファイルを無効化できる", async () => {
    const yamlContent = `
version: "1.0"
activeProfile: "development"
profiles:
  development:
    displayName: "Development"
    env: {}
`;
    await writeFile(
      path.join(testDir, ".gwt", "profiles.yaml"),
      yamlContent,
      "utf-8",
    );

    await setActiveProfile(null);

    const name = await getActiveProfileName();
    expect(name).toBeNull();
  });

  it("存在しないプロファイルを設定しようとするとエラー", async () => {
    const yamlContent = `
version: "1.0"
activeProfile: null
profiles:
  development:
    displayName: "Development"
    env: {}
`;
    await writeFile(
      path.join(testDir, ".gwt", "profiles.yaml"),
      yamlContent,
      "utf-8",
    );

    await expect(setActiveProfile("nonexistent")).rejects.toThrow(
      'Profile "nonexistent" does not exist',
    );
  });
});

describe("createProfile", () => {
  const testDir = path.join(tmpdir(), "gwt-test-profiles-create");

  beforeEach(async () => {
    process.env.GWT_HOME = testDir;
    await mkdir(path.join(testDir, ".gwt"), { recursive: true });
    // resetModules not needed in bun;
    const module = await import("../../../src/config/profiles.js");
    createProfile = module.createProfile;
    loadProfiles = module.loadProfiles;
  });

  afterEach(async () => {
    if (originalGwtHome === undefined) {
      delete process.env.GWT_HOME;
    } else {
      process.env.GWT_HOME = originalGwtHome;
    }
    await rm(testDir, { recursive: true, force: true });
    // resetModules not needed in bun;
  });

  it("新しいプロファイルを作成できる", async () => {
    const newProfile: EnvironmentProfile = {
      displayName: "Staging",
      description: "ステージング環境",
      env: { NODE_ENV: "staging" },
    };

    await createProfile("staging", newProfile);

    const config = await loadProfiles();
    expect(config.profiles.staging).toBeDefined();
    expect(config.profiles.staging.displayName).toBe("Staging");
  });

  it("既存のプロファイル名で作成しようとするとエラー", async () => {
    const yamlContent = `
version: "1.0"
activeProfile: null
profiles:
  development:
    displayName: "Development"
    env: {}
`;
    await writeFile(
      path.join(testDir, ".gwt", "profiles.yaml"),
      yamlContent,
      "utf-8",
    );

    const newProfile: EnvironmentProfile = {
      displayName: "Development",
      env: {},
    };

    await expect(createProfile("development", newProfile)).rejects.toThrow(
      'Profile "development" already exists',
    );
  });

  it("無効なプロファイル名で作成しようとするとエラー", async () => {
    const newProfile: EnvironmentProfile = {
      displayName: "Invalid",
      env: {},
    };

    await expect(createProfile("Invalid Name", newProfile)).rejects.toThrow(
      'Invalid profile name: "Invalid Name". Use lowercase letters, numbers, and hyphens (must start and end with a letter or number).',
    );
  });
});

describe("updateProfile", () => {
  const testDir = path.join(tmpdir(), "gwt-test-profiles-update");

  beforeEach(async () => {
    process.env.GWT_HOME = testDir;
    await mkdir(path.join(testDir, ".gwt"), { recursive: true });
    // resetModules not needed in bun;
    const module = await import("../../../src/config/profiles.js");
    updateProfile = module.updateProfile;
    loadProfiles = module.loadProfiles;
  });

  afterEach(async () => {
    if (originalGwtHome === undefined) {
      delete process.env.GWT_HOME;
    } else {
      process.env.GWT_HOME = originalGwtHome;
    }
    await rm(testDir, { recursive: true, force: true });
    // resetModules not needed in bun;
  });

  it("プロファイルを更新できる", async () => {
    const yamlContent = `
version: "1.0"
activeProfile: null
profiles:
  development:
    displayName: "Development"
    env:
      DEBUG: "true"
`;
    await writeFile(
      path.join(testDir, ".gwt", "profiles.yaml"),
      yamlContent,
      "utf-8",
    );

    await updateProfile("development", {
      displayName: "Dev Environment",
      env: { DEBUG: "false", NEW_VAR: "value" },
    });

    const config = await loadProfiles();
    expect(config.profiles.development.displayName).toBe("Dev Environment");
    expect(config.profiles.development.env.DEBUG).toBe("false");
    expect(config.profiles.development.env.NEW_VAR).toBe("value");
  });

  it("存在しないプロファイルを更新しようとするとエラー", async () => {
    await expect(
      updateProfile("nonexistent", { displayName: "Test" }),
    ).rejects.toThrow('Profile "nonexistent" does not exist');
  });
});

describe("deleteProfile", () => {
  const testDir = path.join(tmpdir(), "gwt-test-profiles-delete");

  beforeEach(async () => {
    process.env.GWT_HOME = testDir;
    await mkdir(path.join(testDir, ".gwt"), { recursive: true });
    // resetModules not needed in bun;
    const module = await import("../../../src/config/profiles.js");
    deleteProfile = module.deleteProfile;
    loadProfiles = module.loadProfiles;
  });

  afterEach(async () => {
    if (originalGwtHome === undefined) {
      delete process.env.GWT_HOME;
    } else {
      process.env.GWT_HOME = originalGwtHome;
    }
    await rm(testDir, { recursive: true, force: true });
    // resetModules not needed in bun;
  });

  it("プロファイルを削除できる", async () => {
    const yamlContent = `
version: "1.0"
activeProfile: null
profiles:
  development:
    displayName: "Development"
    env: {}
  production:
    displayName: "Production"
    env: {}
`;
    await writeFile(
      path.join(testDir, ".gwt", "profiles.yaml"),
      yamlContent,
      "utf-8",
    );

    await deleteProfile("development");

    const config = await loadProfiles();
    expect(config.profiles.development).toBeUndefined();
    expect(config.profiles.production).toBeDefined();
  });

  it("アクティブなプロファイルを削除しようとするとエラー", async () => {
    const yamlContent = `
version: "1.0"
activeProfile: "development"
profiles:
  development:
    displayName: "Development"
    env: {}
`;
    await writeFile(
      path.join(testDir, ".gwt", "profiles.yaml"),
      yamlContent,
      "utf-8",
    );

    await expect(deleteProfile("development")).rejects.toThrow(
      'Cannot delete active profile "development". Please switch to another profile first.',
    );
  });

  it("存在しないプロファイルを削除しようとするとエラー", async () => {
    await expect(deleteProfile("nonexistent")).rejects.toThrow(
      'Profile "nonexistent" does not exist',
    );
  });
});

describe("resolveProfileEnv", () => {
  const testDir = path.join(tmpdir(), "gwt-test-profiles-resolve");

  beforeEach(async () => {
    process.env.GWT_HOME = testDir;
    await mkdir(path.join(testDir, ".gwt"), { recursive: true });
    // resetModules not needed in bun;
    const module = await import("../../../src/config/profiles.js");
    resolveProfileEnv = module.resolveProfileEnv;
  });

  afterEach(async () => {
    if (originalGwtHome === undefined) {
      delete process.env.GWT_HOME;
    } else {
      process.env.GWT_HOME = originalGwtHome;
    }
    await rm(testDir, { recursive: true, force: true });
    // resetModules not needed in bun;
  });

  it("アクティブなプロファイルの環境変数を返す", async () => {
    const yamlContent = `
version: "1.0"
activeProfile: "development"
profiles:
  development:
    displayName: "Development"
    env:
      DEBUG: "true"
      API_KEY: "dev-key"
`;
    await writeFile(
      path.join(testDir, ".gwt", "profiles.yaml"),
      yamlContent,
      "utf-8",
    );

    const env = await resolveProfileEnv();
    expect(env.DEBUG).toBe("true");
    expect(env.API_KEY).toBe("dev-key");
  });

  it("アクティブなプロファイルがない場合、空オブジェクトを返す", async () => {
    const yamlContent = `
version: "1.0"
activeProfile: null
profiles: {}
`;
    await writeFile(
      path.join(testDir, ".gwt", "profiles.yaml"),
      yamlContent,
      "utf-8",
    );

    const env = await resolveProfileEnv();
    expect(env).toEqual({});
  });

  it("設定ファイルが存在しない場合、空オブジェクトを返す", async () => {
    const env = await resolveProfileEnv();
    expect(env).toEqual({});
  });
});
