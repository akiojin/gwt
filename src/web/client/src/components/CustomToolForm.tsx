import React, { useState } from "react";
import type {
  CustomAITool,
  EnvironmentVariable,
} from "../../../../types/api.js";

export interface CustomToolFormValue {
  id: string;
  displayName: string;
  icon?: string | null;
  description?: string | null;
  executionType: CustomAITool["executionType"];
  command: string;
  defaultArgs?: string[] | null;
  modeArgs: CustomAITool["modeArgs"];
  permissionSkipArgs?: string[] | null;
  env?: EnvironmentVariable[] | null;
}

interface CustomToolFormProps {
  initialValue?: CustomAITool;
  onSubmit: (value: CustomToolFormValue) => void;
  onCancel: () => void;
  isSaving?: boolean;
}

interface FormErrors {
  id?: string;
  displayName?: string;
  command?: string;
  env?: string;
}

export function CustomToolForm({ initialValue, onSubmit, onCancel, isSaving }: CustomToolFormProps) {
  const [formState, setFormState] = useState(() => createInitialState(initialValue));
  const [errors, setErrors] = useState<FormErrors>({});

  const title = initialValue ? "ツールを編集" : "新規カスタムツール";

  const handleChange = (field: keyof typeof formState) =>
    (event: React.ChangeEvent<HTMLInputElement | HTMLTextAreaElement | HTMLSelectElement>) => {
      setFormState((prev) => ({ ...prev, [field]: event.target.value }));
    };

  const handleSubmit = (event: React.FormEvent) => {
    event.preventDefault();
    const nextErrors: FormErrors = {};

    if (!formState.id.trim()) {
      nextErrors.id = "IDは必須です";
    }
    if (!formState.displayName.trim()) {
      nextErrors.displayName = "表示名は必須です";
    }
    if (!formState.command.trim()) {
      nextErrors.command = "コマンド/パッケージ名は必須です";
    }

    const envResult = parseEnv(formState.env);
    let parsedEnv: EnvironmentVariable[] | null = null;
    if (envResult instanceof Error) {
      nextErrors.env = envResult.message;
    } else {
      parsedEnv = envResult;
    }

    if (Object.keys(nextErrors).length) {
      setErrors(nextErrors);
      return;
    }

    setErrors({});
    onSubmit({
      id: formState.id.trim(),
      displayName: formState.displayName.trim(),
      icon: formState.icon?.trim() ? formState.icon.trim() : null,
      description: formState.description?.trim() ? formState.description.trim() : null,
      executionType: formState.executionType,
      command: formState.command.trim(),
      defaultArgs: parseList(formState.defaultArgs),
      modeArgs: {
        normal: parseList(formState.modeNormal) ?? [],
        continue: parseList(formState.modeContinue) ?? [],
        resume: parseList(formState.modeResume) ?? [],
      },
      permissionSkipArgs: parseList(formState.permissionSkipArgs),
      env: parsedEnv,
    });
  };

  return (
    <form className="tool-form" onSubmit={handleSubmit}>
      <div className="tool-form__header">
        <div>
          <p className="tool-card__eyebrow">{title}</p>
          <h3>{formState.displayName || "カスタムAIツール"}</h3>
        </div>
        <div className="tool-form__controls">
          <button type="button" className="button button--ghost" onClick={onCancel} disabled={isSaving}>
            キャンセル
          </button>
          <button type="submit" className="button button--primary" disabled={isSaving}>
            {isSaving ? "保存中..." : "保存"}
          </button>
        </div>
      </div>

      <div className="form-grid">
        <label className="form-field">
          <span>ツールID *</span>
          <input
            type="text"
            value={formState.id}
            onChange={handleChange("id")}
            disabled={Boolean(initialValue)}
          />
          {errors.id && <p className="form-field__error">{errors.id}</p>}
        </label>

        <label className="form-field">
          <span>表示名 *</span>
          <input type="text" value={formState.displayName} onChange={handleChange("displayName")} />
          {errors.displayName && <p className="form-field__error">{errors.displayName}</p>}
        </label>

        <label className="form-field">
          <span>アイコン (任意)</span>
          <input type="text" value={formState.icon} onChange={handleChange("icon")} maxLength={2} />
        </label>

        <label className="form-field">
          <span>説明 (任意)</span>
          <input type="text" value={formState.description} onChange={handleChange("description")} />
        </label>

        <label className="form-field">
          <span>実行タイプ *</span>
          <select value={formState.executionType} onChange={handleChange("executionType")}
            disabled={Boolean(initialValue)}>
            <option value="path">path (絶対パス)</option>
            <option value="bunx">bunx (パッケージ)</option>
            <option value="command">command (PATH)</option>
          </select>
        </label>

        <label className="form-field">
          <span>{formState.executionType === "bunx" ? "パッケージ名" : "コマンド"} *</span>
          <input type="text" value={formState.command} onChange={handleChange("command")} />
          {errors.command && <p className="form-field__error">{errors.command}</p>}
        </label>
      </div>

      <div className="form-grid">
        <label className="form-field form-field--stacked">
          <span>defaultArgs (改行区切り)</span>
          <textarea value={formState.defaultArgs} onChange={handleChange("defaultArgs")} rows={2} />
        </label>

        <label className="form-field form-field--stacked">
          <span>permissionSkipArgs (改行区切り)</span>
          <textarea
            value={formState.permissionSkipArgs}
            onChange={handleChange("permissionSkipArgs")} rows={2}
          />
        </label>
      </div>

      <div className="form-grid form-grid--thirds">
        <label className="form-field form-field--stacked">
          <span>normalモード引数</span>
          <textarea value={formState.modeNormal} onChange={handleChange("modeNormal")} rows={3} />
        </label>
        <label className="form-field form-field--stacked">
          <span>continueモード引数</span>
          <textarea value={formState.modeContinue} onChange={handleChange("modeContinue")} rows={3} />
        </label>
        <label className="form-field form-field--stacked">
          <span>resumeモード引数</span>
          <textarea value={formState.modeResume} onChange={handleChange("modeResume")} rows={3} />
        </label>
      </div>

      <label className="form-field form-field--stacked">
        <span>環境変数 (key=value を改行で記述)</span>
        <textarea value={formState.env} onChange={handleChange("env")} rows={3} />
        {errors.env && <p className="form-field__error">{errors.env}</p>}
      </label>
    </form>
  );
}

function createInitialState(initialValue?: CustomAITool) {
  if (!initialValue) {
    return {
      id: "",
      displayName: "",
      icon: "",
      description: "",
      executionType: "bunx" as CustomAITool["executionType"],
      command: "",
      defaultArgs: "",
      modeNormal: "",
      modeContinue: "",
      modeResume: "",
      permissionSkipArgs: "",
      env: "",
    };
  }

  return {
    id: initialValue.id,
    displayName: initialValue.displayName,
    icon: initialValue.icon ?? "",
    description: initialValue.description ?? "",
    executionType: initialValue.executionType,
    command: initialValue.command,
    defaultArgs: joinList(initialValue.defaultArgs),
    modeNormal: joinList(initialValue.modeArgs?.normal),
    modeContinue: joinList(initialValue.modeArgs?.continue),
    modeResume: joinList(initialValue.modeArgs?.resume),
    permissionSkipArgs: joinList(initialValue.permissionSkipArgs),
    env: stringifyEnv(initialValue.env),
  };
}

function joinList(values?: string[] | null): string {
  if (!values || values.length === 0) {
    return "";
  }
  return values.join("\n");
}

function parseList(value: string): string[] | null {
  if (!value.trim()) {
    return null;
  }
  const values = value
    .split(/\n|,/)
    .map((item) => item.trim())
    .filter(Boolean);
  return values.length ? values : null;
}

function stringifyEnv(env?: EnvironmentVariable[] | null): string {
  if (!env || env.length === 0) {
    return "";
  }
  return env
    .filter((variable) => variable.key)
    .map((variable) => `${variable.key}=${variable.value}`)
    .join("\n");
}

function parseEnv(value: string): EnvironmentVariable[] | null | Error {
  if (!value.trim()) {
    return null;
  }

  const now = new Date().toISOString();
  const result: EnvironmentVariable[] = [];
  const seen = new Set<string>();
  const lines = value
    .split(/\n/)
    .map((line) => line.trim())
    .filter(Boolean);

  for (const line of lines) {
    const [rawKey, ...rest] = line.split("=");
    const key = (rawKey ?? "").trim();
    if (!key || rest.length === 0) {
      return new Error("環境変数は key=value 形式で入力してください");
    }
    const val = rest.join("=").trim();
    if (!val) {
      return new Error(`${key} の値を入力してください`);
    }
    if (seen.has(key)) {
      return new Error(`環境変数 ${key} が重複しています`);
    }
    seen.add(key);
    result.push({
      key,
      value: val,
      lastUpdated: now,
    });
  }

  return result;
}
