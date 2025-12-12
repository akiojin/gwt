import React, { useState } from "react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
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

export function CustomToolForm({
  initialValue,
  onSubmit,
  onCancel,
  isSaving,
}: CustomToolFormProps) {
  const [formState, setFormState] = useState(() =>
    createInitialState(initialValue),
  );
  const [errors, setErrors] = useState<FormErrors>({});

  const title = initialValue ? "ツールを編集" : "新規カスタムツール";

  const handleChange =
    (field: keyof typeof formState) =>
    (event: React.ChangeEvent<HTMLInputElement | HTMLTextAreaElement>) => {
      setFormState((prev) => ({ ...prev, [field]: event.target.value }));
    };

  const handleSelectChange =
    (field: keyof typeof formState) => (value: string) => {
      setFormState((prev) => ({ ...prev, [field]: value }));
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
      description: formState.description?.trim()
        ? formState.description.trim()
        : null,
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
    <form onSubmit={handleSubmit} className="space-y-6">
      <div className="flex items-start justify-between gap-4">
        <div>
          <p className="text-xs font-medium uppercase tracking-wider text-muted-foreground">
            {title}
          </p>
          <h3 className="mt-1 text-lg font-semibold">
            {formState.displayName || "カスタムAIツール"}
          </h3>
        </div>
        <div className="flex gap-2">
          <Button
            type="button"
            variant="ghost"
            onClick={onCancel}
            disabled={isSaving}
          >
            キャンセル
          </Button>
          <Button type="submit" disabled={isSaving}>
            {isSaving ? "保存中..." : "保存"}
          </Button>
        </div>
      </div>

      <div className="grid gap-4 sm:grid-cols-2">
        <div className="space-y-2">
          <label className="text-sm font-medium">ツールID *</label>
          <Input
            type="text"
            value={formState.id}
            onChange={handleChange("id")}
            disabled={Boolean(initialValue)}
          />
          {errors.id && <p className="text-xs text-destructive">{errors.id}</p>}
        </div>

        <div className="space-y-2">
          <label className="text-sm font-medium">表示名 *</label>
          <Input
            type="text"
            value={formState.displayName}
            onChange={handleChange("displayName")}
          />
          {errors.displayName && (
            <p className="text-xs text-destructive">{errors.displayName}</p>
          )}
        </div>

        <div className="space-y-2">
          <label className="text-sm font-medium">アイコン (任意)</label>
          <Input
            type="text"
            value={formState.icon}
            onChange={handleChange("icon")}
            maxLength={2}
          />
        </div>

        <div className="space-y-2">
          <label className="text-sm font-medium">説明 (任意)</label>
          <Input
            type="text"
            value={formState.description}
            onChange={handleChange("description")}
          />
        </div>

        <div className="space-y-2">
          <label className="text-sm font-medium">実行タイプ *</label>
          <Select
            value={formState.executionType}
            onValueChange={handleSelectChange("executionType")}
            disabled={Boolean(initialValue)}
          >
            <SelectTrigger>
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="path">path (絶対パス)</SelectItem>
              <SelectItem value="bunx">bunx (パッケージ)</SelectItem>
              <SelectItem value="command">command (PATH)</SelectItem>
            </SelectContent>
          </Select>
        </div>

        <div className="space-y-2">
          <label className="text-sm font-medium">
            {formState.executionType === "bunx" ? "パッケージ名" : "コマンド"} *
          </label>
          <Input
            type="text"
            value={formState.command}
            onChange={handleChange("command")}
          />
          {errors.command && (
            <p className="text-xs text-destructive">{errors.command}</p>
          )}
        </div>
      </div>

      <div className="grid gap-4 sm:grid-cols-2">
        <div className="space-y-2">
          <label className="text-sm font-medium">
            defaultArgs (改行区切り)
          </label>
          <textarea
            value={formState.defaultArgs}
            onChange={handleChange("defaultArgs")}
            rows={2}
            className="flex w-full rounded-md border border-border bg-background px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-primary"
          />
        </div>

        <div className="space-y-2">
          <label className="text-sm font-medium">
            permissionSkipArgs (改行区切り)
          </label>
          <textarea
            value={formState.permissionSkipArgs}
            onChange={handleChange("permissionSkipArgs")}
            rows={2}
            className="flex w-full rounded-md border border-border bg-background px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-primary"
          />
        </div>
      </div>

      <div className="grid gap-4 sm:grid-cols-3">
        <div className="space-y-2">
          <label className="text-sm font-medium">normalモード引数</label>
          <textarea
            value={formState.modeNormal}
            onChange={handleChange("modeNormal")}
            rows={3}
            className="flex w-full rounded-md border border-border bg-background px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-primary"
          />
        </div>
        <div className="space-y-2">
          <label className="text-sm font-medium">continueモード引数</label>
          <textarea
            value={formState.modeContinue}
            onChange={handleChange("modeContinue")}
            rows={3}
            className="flex w-full rounded-md border border-border bg-background px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-primary"
          />
        </div>
        <div className="space-y-2">
          <label className="text-sm font-medium">resumeモード引数</label>
          <textarea
            value={formState.modeResume}
            onChange={handleChange("modeResume")}
            rows={3}
            className="flex w-full rounded-md border border-border bg-background px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-primary"
          />
        </div>
      </div>

      <div className="space-y-2">
        <label className="text-sm font-medium">
          環境変数 (key=value を改行で記述)
        </label>
        <textarea
          value={formState.env}
          onChange={handleChange("env")}
          rows={3}
          className="flex w-full rounded-md border border-border bg-background px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-primary"
        />
        {errors.env && <p className="text-xs text-destructive">{errors.env}</p>}
      </div>
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
