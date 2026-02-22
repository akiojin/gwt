# 実装計画: Error Reporting & Feature Suggestion

**仕様ID**: `SPEC-fabb6678` | **日付**: 2026-02-22 | **仕様書**: `specs/SPEC-fabb6678/spec.md`

## 目的

- gwt 利用中の問題報告・改善提案を GitHub Issues へ直接起票できるようにする
- エラーハンドリングを構造化し、予期しないエラーの検知・通知・報告を統一的に行う
- スクリーンキャプチャ（テキスト+画像）と診断情報を報告に添付できるようにする

## 技術コンテキスト

- **バックエンド**: Rust 2021 + Tauri v2（`crates/gwt-tauri/`, `crates/gwt-core/`）
- **フロントエンド**: Svelte 5 + TypeScript（`gwt-gui/`）
- **既存資産**:
  - `GwtError` enum with E-codes (E1xxx-E9xxx), `ErrorCategory`, `code()`, `message()`, `suggestions()`
  - `screenCapture.ts` の `collectScreenText()`
  - Toast 通知システム（App.svelte, 現在はアップデート通知のみ）
  - ネイティブメニューシステム（`menu.rs`）
  - GitHub Integration（SettingsPanel, プロファイル別 AI 設定）
- **テスト**: cargo test / vitest
- **前提**:
  - Tauri コマンドの戻り値を `Result<T, String>` から `Result<T, StructuredError>` に移行
  - macOS スクリーンキャプチャには `CGWindowListCreateImage` API を使用（スクリーン収録権限が必要）
  - Windows スクリーンキャプチャには `PrintWindow` / `BitBlt` API を使用

## 実装方針

### Phase 1: 構造化エラー型 + エラーバス基盤

**目的**: エラーの分類・通知・報告の基盤を構築する

#### 1-1. Rust 側: 構造化エラー応答型の導入

**対象ファイル**: `crates/gwt-core/src/error.rs`, `crates/gwt-tauri/src/commands/*.rs`

既存の `GwtError` には `code()`, `category()`, `message()`, `suggestions()` が揃っている。
これを Tauri コマンドのエラー応答として構造化して返す。

```rust
// crates/gwt-core/src/error.rs に追加
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ErrorSeverity {
    Info,      // ユーザー操作エラー（ブランチ名重複等）→ トースト不要
    Warning,   // 注意が必要だが致命的でない → トースト不要
    Error,     // 予期しないエラー → トースト表示
    Critical,  // アプリ動作に影響する致命的エラー → トースト表示
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StructuredError {
    pub severity: ErrorSeverity,
    pub code: String,           // "E1001" etc.
    pub message: String,        // 人間可読メッセージ
    pub command: String,        // 呼び出されたコマンド名
    pub category: String,       // ErrorCategory の文字列表現
    pub suggestions: Vec<String>,
    pub timestamp: String,      // ISO 8601
}
```

severity の判定ルール:
- `Info`: ユーザー入力に起因するバリデーションエラー（E1xxx の一部、E2xxx の一部）
- `Warning`: リカバリ可能だが注意が必要なエラー
- `Error`: 予期しない内部エラー（E9xxx）、外部サービスエラー（E5xxx）
- `Critical`: データ破損の可能性があるエラー

既存の `GwtError` に `severity()` メソッドを追加し、各バリアントに対してデフォルトの severity を返す。

#### 1-2. Tauri コマンドのエラー応答を移行

**対象ファイル**: `crates/gwt-tauri/src/commands/*.rs`（全27モジュール）

現在の `Result<T, String>` を `Result<T, StructuredError>` に移行する。
Tauri v2 では `impl serde::Serialize` を満たす型をエラーとして返せるため、`StructuredError` を直接使用可能。

移行パターン:
```rust
// Before
#[tauri::command]
pub async fn list_worktrees(project_path: String) -> Result<Vec<WorktreeInfo>, String> {
    // ... .map_err(|e| e.to_string())
}

// After
#[tauri::command]
pub async fn list_worktrees(project_path: String) -> Result<Vec<WorktreeInfo>, StructuredError> {
    // ... .map_err(|e| StructuredError::from_gwt_error(e, "list_worktrees"))
}
```

`StructuredError::from_gwt_error(error: GwtError, command: &str) -> StructuredError` を実装し、
既存の `GwtError` から自動変換する。

#### 1-3. フロントエンド: エラーバスとラッパー関数

**対象ファイル**: `gwt-gui/src/lib/errorBus.ts`（新規）, `gwt-gui/src/lib/tauriInvoke.ts`（新規）

```typescript
// errorBus.ts — グローバルエラーバス
export interface StructuredError {
  severity: "info" | "warning" | "error" | "critical";
  code: string;
  message: string;
  command: string;
  category: string;
  suggestions: string[];
  timestamp: string;
}

type ErrorHandler = (error: StructuredError) => void;

class ErrorBus {
  private handlers: ErrorHandler[] = [];
  private sessionFingerprints = new Set<string>();

  subscribe(handler: ErrorHandler): () => void { ... }
  emit(error: StructuredError): void { ... }
  isSupressed(error: StructuredError): boolean { ... }
}

export const errorBus = new ErrorBus();
```

```typescript
// tauriInvoke.ts — invoke() ラッパー
import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { errorBus } from "./errorBus";

export async function invoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await tauriInvoke<T>(command, args);
  } catch (err) {
    const structured = parseStructuredError(err, command);
    errorBus.emit(structured);
    throw structured; // 既存の catch ブロックでも処理可能にする
  }
}
```

#### 1-4. 既存コンポーネントの invoke() 移行

**対象**: 全 Svelte コンポーネント + TS ユーティリティ

既存の `import { invoke } from "@tauri-apps/api/core"` を
`import { invoke } from "$lib/tauriInvoke"` に置き換える。

既存の各コンポーネントの try/catch は維持する（インライン表示は変えない）。
エラーバスは「追加の通知経路」として機能し、既存動作を壊さない。

#### 1-5. トースト通知の拡張

**対象ファイル**: `gwt-gui/src/App.svelte`

既存の toast システムを拡張し、エラー報告用のアクションを追加する。

```typescript
type ToastAction =
  | { kind: "apply-update"; latest: string }
  | { kind: "report-error"; error: StructuredError }
  | null;
```

エラーバスを App.svelte で購読し、severity=error/critical のエラーを検知したらトースト表示。
トーストに "Report" リンクを表示し、クリックで報告モーダルを開く。

---

### Phase 2: 報告フォーム UI + GitHub Issues 連携

**目的**: 統合報告モーダルと GitHub Issues 自動起票を実装する

#### 2-1. 報告モーダルコンポーネント

**対象ファイル**: `gwt-gui/src/lib/components/ReportDialog.svelte`（新規）

統合報告モーダル:
- "Bug Report" / "Feature Request" のタブ切り替え
- Bug Report: Title, Steps to Reproduce, Expected Result, Actual Result
- Feature Request: Title, Description, Use Case, Expected Benefit
- 送信先リポジトリ ドロップダウン（akiojin/gwt + 作業中リポ）
- 診断情報チェックボックス（System Info / Application Logs / Screen Capture）
- Preview ボタン + プレビューエリア（編集可能）
- Submit ボタン + フォールバック時の "Copy & Open in Browser" ボタン

エラー自動検知から開かれた場合: `prefillError: StructuredError` prop でプリフィル。

#### 2-2. プライバシーマスキングユーティリティ

**対象ファイル**: `gwt-gui/src/lib/privacyMask.ts`（新規）

マスキング対象パターン（正規表現）:
- `sk-ant-[A-Za-z0-9_-]+` → `[REDACTED:API_KEY]`
- `sk-[A-Za-z0-9_-]{20,}` → `[REDACTED:API_KEY]`
- `ghp_[A-Za-z0-9]{36,}` → `[REDACTED:GITHUB_TOKEN]`
- `gho_[A-Za-z0-9]{36,}` → `[REDACTED:GITHUB_TOKEN]`
- `github_pat_[A-Za-z0-9_]{20,}` → `[REDACTED:GITHUB_PAT]`
- `Bearer [A-Za-z0-9_.-]+` → `Bearer [REDACTED]`
- `password\s*[:=]\s*\S+` → `password: [REDACTED]`
- `[A-Za-z_]*(?:KEY|TOKEN|SECRET|PASSWORD)[A-Za-z_]*\s*[:=]\s*\S+` → `...: [REDACTED]`

#### 2-3. 診断情報収集

**対象ファイル**: `gwt-gui/src/lib/diagnostics.ts`（新規）, `crates/gwt-tauri/src/commands/report.rs`（新規）

収集項目:
- **System Info**: OS, バージョン, CPU, メモリ（既存の `get_system_info` を流用）
- **Application Logs**: `~/.gwt/logs/` の最新200行（Rust 側で読み取り）
- **Screen Capture**: `collectScreenText()` の結果（テキスト）

Rust 側に新コマンドを追加:
```rust
#[tauri::command]
pub async fn read_recent_logs(max_lines: usize) -> Result<String, StructuredError> { ... }

#[tauri::command]
pub async fn get_report_system_info() -> Result<ReportSystemInfo, StructuredError> { ... }
```

#### 2-4. GitHub Issues 連携

**対象ファイル**: `crates/gwt-tauri/src/commands/report.rs`（新規）

Rust 側で GitHub API を呼び出して Issue を作成する。

```rust
#[tauri::command]
pub async fn create_github_issue(
    repo: String,         // "owner/repo"
    title: String,
    body: String,
    labels: Vec<String>,
) -> Result<CreatedIssue, StructuredError> { ... }
```

認証: プロジェクト設定の GitHub トークンまたは `gh` CLI の認証情報を使用。
フォールバック: エラー時はフロントエンドでクリップボードコピー + `shell.open()` でブラウザ起動。

#### 2-5. Issue テンプレート生成

**対象ファイル**: `gwt-gui/src/lib/issueTemplate.ts`（新規）

Bug Report テンプレート:
```markdown
## Bug Report

### Steps to Reproduce
{steps}

### Expected Result
{expected}

### Actual Result
{actual}

---

### Diagnostic Information

#### System
- OS: {os}
- gwt Version: {version}
- Platform: {platform}

#### Error Details
- Code: {error_code}
- Command: {command}
- Timestamp: {timestamp}

#### Screen Capture
{screen_text_or_image_path}

#### Application Logs (last {n} lines)
```
{logs}
```
```

Feature Request テンプレート:
```markdown
## Feature Request

### Description
{description}

### Use Case
{use_case}

### Expected Benefit
{benefit}

---

### Context
- gwt Version: {version}
- Platform: {platform}
```

---

### Phase 3: スクリーンキャプチャ + Help メニュー

**目的**: OS ネイティブ画像キャプチャと Help メニュー導線を実装する

#### 3-1. OS ネイティブスクリーンキャプチャ（Rust）

**対象ファイル**: `crates/gwt-core/src/screenshot.rs`（新規）, `crates/gwt-tauri/src/commands/report.rs`

macOS:
```rust
// CGWindowListCreateImage を使用して gwt ウィンドウをキャプチャ
// core-graphics crate を使用
fn capture_window_macos(window_id: u32) -> Result<Vec<u8>, GwtError> { ... }
```

Windows:
```rust
// PrintWindow / BitBlt API を使用
// windows crate を使用
fn capture_window_windows(hwnd: HWND) -> Result<Vec<u8>, GwtError> { ... }
```

Tauri コマンド:
```rust
#[tauri::command]
pub async fn capture_screenshot(
    window: tauri::Window,
    save_dir: String,
) -> Result<String, StructuredError> {
    // 1. ウィンドウの OS ネイティブハンドルを取得
    // 2. OS API でキャプチャ
    // 3. ~/.gwt/reports/images/{timestamp}.png に保存
    // 4. 保存パスを返す
}
```

#### 3-2. Help メニューの追加

**対象ファイル**: `crates/gwt-tauri/src/menu.rs`

```rust
pub const MENU_ID_HELP_REPORT_ISSUE: &str = "help-report-issue";
pub const MENU_ID_HELP_SUGGEST_FEATURE: &str = "help-suggest-feature";

// Help メニューを構築
let help = SubmenuBuilder::new(app, "Help")
    .item(&MenuItem::with_id(
        app, MENU_ID_HELP_REPORT_ISSUE,
        "Report Issue...", true, Some("CmdOrCtrl+Shift+R")
    )?)
    .item(&MenuItem::with_id(
        app, MENU_ID_HELP_SUGGEST_FEATURE,
        "Suggest Feature...", true, None::<&str>
    )?)
    .separator()
    .item(&help_about)
    .item(&help_check_updates)
    .build()?;
```

メニューの順序: App / File / Edit / Git / Tools / Window / **Help**

About と Check for Updates を Help メニューに移動する（macOS の Application メニューからは About のみ残す）。

#### 3-3. メニューイベント処理

**対象ファイル**: `gwt-gui/src/App.svelte`

```typescript
// menu-action イベントハンドラに追加
case "help-report-issue":
  showReportDialog("bug");
  break;
case "help-suggest-feature":
  showReportDialog("feature");
  break;
```

## テスト

### バックエンド

- `StructuredError::from_gwt_error()` の変換テスト: 各 ErrorCategory × severity の組み合わせ
- `severity()` メソッド: 全 GwtError バリアントの severity が正しく判定されること
- `read_recent_logs()`: ログファイルが存在しない場合・空の場合・200行超の場合
- `capture_screenshot()`: ウィンドウハンドル取得・画像保存のユニットテスト
- `create_github_issue()`: API レスポンスのパース・エラーハンドリング

### フロントエンド

- `ErrorBus`: subscribe/emit/セッション重複抑制のユニットテスト
- `tauriInvoke`: エラー発生時にエラーバスに通知されることのテスト
- `privacyMask`: 各マスキングパターンのユニットテスト（API キー、トークン、パスワード等）
- `issueTemplate`: Bug Report / Feature Request テンプレート生成のテスト
- `ReportDialog.svelte`: タブ切り替え、フィールド表示切り替え、Submit / Preview のテスト
- `diagnostics.ts`: 診断情報収集のテスト

## リスク・依存関係

| リスク | 影響 | 緩和策 |
|--------|------|--------|
| 全 invoke() 移行の既存動作への影響 | 高 | ラッパーは既存の throw を維持。エラーバスは追加の通知経路のみ |
| macOS スクリーン収録権限の UX | 中 | 権限未許可時はテキストキャプチャへフォールバック |
| GitHub API 認証の複雑さ | 中 | `gh` CLI のトークンを流用。未認証時はブラウザフォールバック |
| 27コマンドモジュールの一括移行 | 高 | 機械的な置換が可能。`from_gwt_error` で自動変換 |

## マイルストーン

| Phase | 内容 | 依存関係 |
|-------|------|----------|
| Phase 1 | 構造化エラー + エラーバス + トースト | なし |
| Phase 2 | 報告フォーム + GitHub Issues + マスキング | Phase 1 |
| Phase 3 | スクリーンキャプチャ + Help メニュー | Phase 2 |
