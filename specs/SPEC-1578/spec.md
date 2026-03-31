> **📜 HISTORICAL (SPEC-1776)**: This SPEC was written for the previous GUI stack (Tauri/Svelte/C#). It is retained as a historical reference. The gwt-tui migration (SPEC-1776) supersedes GUI-specific design decisions described here.

### 背景

gwt はプロジェクトを開いた際、AI エージェント（Claude Code, Codex, Gemini 等）用のスキル・コマンド・フック・設定ファイルを対象プロジェクトのローカルディレクトリに自動配置する。これらのファイルはユーザーのリポジトリに直接書き込まれるが、**リポジトリの Git 履歴には含めるべきでない**（gwt 固有のファイルであり、プロジェクトのソースコードではない）。

そのため、gwt は `.git/info/exclude` にマネージドブロック（`# BEGIN gwt managed local assets` 〜 `# END gwt managed local assets`）を書き込み、gwt 管理ファイルを Git 追跡対象から除外する。

**現行 Rust 実装**: `crates/gwt-core/src/config/skill_registration.rs` の `ensure_project_local_exclude_rules` 関数で実装済み。Unity C# への移植が必要。

### 現行の除外パターン

```text
# BEGIN gwt managed local assets
/.codex/skills/gwt-*/
/.gemini/skills/gwt-*/
/.claude/skills/gwt-*/
/.claude/commands/gwt-*.md
/.claude/hooks/scripts/gwt-*.sh
/.claude/settings.local.json
# END gwt managed local assets
```

### レガシーパターン（移行時に除去）

```text
.gwt/
/.gwt/
/.codex/skills/gwt-*/**
```

### ユーザーシナリオ

| ID | シナリオ | 優先度 |
|----|---------|--------|
| US-1 | ユーザーがプロジェクトを gwt で開くと、AI エージェント用スキル/コマンド/フックが自動的にプロジェクトディレクトリに配置される | P0 |
| US-2 | 配置されたファイルが `.git/info/exclude` により Git 追跡から除外され、`git status` に表示されない | P0 |
| US-3 | 既存の `.git/info/exclude` にユーザー独自のルールがある場合、それらが保持される（gwt マネージドブロックのみ差し替え） | P0 |
| US-4 | gwt のバージョンアップでスキルファイルの内容が更新された場合、プロジェクトを開き直すと最新版に上書き更新される | P0 |
| US-5 | worktree（リンク先）でプロジェクトを開いた場合、exclude ルールが commondir（メインリポジトリの `.git`）に書き込まれる | P0 |
| US-6 | レガシーパターン（`/.codex/skills/gwt-*/**` 等）が含まれる場合、新パターンに自動移行される | P1 |
| US-7 | マネージドブロックのマーカーが不正（入れ子、終端なし等）の場合、エラーを返しサイレントに壊さない | P0 |
| US-8 | Claude Code の `settings.local.json` に gwt 管理のフック定義が自動登録される | P0 |
| US-9 | `.claude/settings.json` や `~/.claude/settings.json` が存在しても、gwt はそれらを参照・改変せず `.claude/settings.local.json` だけを使う | P0 |
| US-10 | Claude hook が repo ルート以外の CWD（例: `gwt-gui/`、Docker/DevContainer 内の workspace）から起動されても、同じプロジェクト配下の `.claude/hooks/scripts/` を解決できる | P0 |

### US-1 詳細

1. ユーザーが gwt でプロジェクトを開く（またはエージェント登録 API を呼び出す）
2. gwt が対象プロジェクトの `.claude/skills/`, `.codex/skills/`, `.gemini/skills/` 等にスキルファイルを書き込む
3. スキルファイルの内容はバイナリに埋め込まれたテンプレート（`include_str!` 相当）から生成される
4. `${CLAUDE_PLUGIN_ROOT}` 等のプレースホルダがプロジェクトルート名に置換される

### US-2 詳細

1. スキル配置後、`ensure_project_local_exclude_rules` が呼ばれる
2. `.git/info/exclude` を読み込み、既存のマネージドブロックがあれば除去する
3. ユーザー独自のルールを保持しつつ、末尾にマネージドブロックを追記する
4. `git status` で gwt 管理ファイルが表示されないことを確認

### US-5 詳細

1. ユーザーが worktree 内でプロジェクトを開く
2. gwt が `git rev-parse --git-common-dir` でメインリポジトリの `.git` パスを解決する
3. exclude ルールをメインリポジトリの `.git/info/exclude` に書き込む（worktree の `.git` ファイルではなく）
4. これにより全 worktree で統一的に gwt ファイルが除外される

### US-7 詳細

1. `.git/info/exclude` に `# BEGIN` マーカーがあるが `# END` がない場合
2. `ensure_project_local_exclude_rules` がエラーを返す（`Malformed managed exclude block`）
3. ファイルは変更されない（安全側に倒す）

### 機能要件

| ID | 要件 | 関連US |
|----|------|--------|
| FR-001 | プロジェクトオープン時に AI エージェント用スキル/コマンド/フックファイルをプロジェクトディレクトリに自動配置する | US-1 |
| FR-002 | スキルファイルの内容はバイナリ埋め込み（Unity の `TextAsset` またはリソース）から生成する | US-1, US-4 |
| FR-003 | `${CLAUDE_PLUGIN_ROOT}` 等のプレースホルダをプロジェクトルート名に実行時置換する | US-1 |
| FR-004 | `.git/info/exclude` にマネージドブロック（`# BEGIN gwt managed local assets` 〜 `# END gwt managed local assets`）を書き込む | US-2 |
| FR-005 | マネージドブロック外の既存ルールを保持する（ユーザー独自ルールの破壊禁止） | US-3 |
| FR-006 | マネージドブロックの差し替えは冪等（何回実行しても同じ結果） | US-2 |
| FR-007 | gwt バージョンアップ時、スキルファイルの内容を最新版に上書き更新する | US-4 |
| FR-008 | worktree 環境では `git rev-parse --git-common-dir` で commondir を解決し、メインリポジトリの `.git/info/exclude` に書き込む | US-5 |
| FR-009 | レガシーパターン（`/.codex/skills/gwt-*/**`, `.gwt/`, `/.gwt/`）を検出した場合、新パターンに自動移行する（レガシー行を除去し、マネージドブロックに統一） | US-6 |
| FR-010 | マネージドブロックのマーカーが不正（入れ子 BEGIN、END なし BEGIN、BEGIN なし END）の場合、エラーを返しファイルを変更しない | US-7 |
| FR-011 | Claude Code の `.claude/settings.local.json` に gwt 管理のフック定義（`UserPromptSubmit`, `PreToolUse`, `PostToolUse`）を自動登録する | US-8 |
| FR-012 | `.claude/settings.local.json` のフック登録も冪等で、既存の非 gwt フック設定を破壊しない | US-8 |
| FR-013 | UNIX 環境ではスクリプトファイル（`.sh`）に実行権限（0o755）を付与する | US-1 |
| FR-014 | register 時に `.claude/settings.json` と `~/.claude/settings.json` を参照・改変しない | US-9 |
| FR-015 | Claude Code の gwt 管理 hook コマンドは CWD 非依存でなければならず、repo ルートを実行時に解決して `.claude/hooks/scripts/` を参照する | US-10 |
| FR-016 | Docker / DevContainer では、コンテナ内で Git worktree と `git` コマンドが利用可能な限り、mount 先の絶対パスに依存せず同じ hook 定義で動作しなければならない | US-10 |

### 除外パターン一覧

| パターン | 対象 |
|---------|------|
| `/.codex/skills/gwt-*/` | Codex 用 gwt スキル |
| `/.gemini/skills/gwt-*/` | Gemini 用 gwt スキル |
| `/.claude/skills/gwt-*/` | Claude Code 用 gwt スキル |
| `/.claude/commands/gwt-*.md` | Claude Code 用 gwt コマンド |
| `/.claude/hooks/scripts/gwt-*.sh` | Claude Code 用 gwt フックスクリプト |
| `/.claude/settings.local.json` | Claude Code 用 gwt 設定（ローカル） |

### 配置アセット一覧

| 配置先パス | 内容 | プレースホルダ置換 |
|-----------|------|------------------|
| `.claude/skills/gwt-*/SKILL.md` | 各スキル定義 | あり（`${CLAUDE_PLUGIN_ROOT}`） |
| `.claude/commands/gwt-*.md` | 各コマンド定義 | あり |
| `.claude/hooks/scripts/gwt-*.sh` | フックスクリプト | なし |
| `.claude/settings.local.json` | フック定義（マージ） | なし |
| `.codex/skills/gwt-*/SKILL.md` | Codex 用スキル定義 | なし |
| `.gemini/skills/gwt-*/SKILL.md` | Gemini 用スキル定義 | なし |

### 非機能要件

| ID | 要件 |
|----|------|
| NFR-001 | スキル登録処理の実行時間: 全アセット配置 + exclude ルール書き込みで 2 秒以内 |
| NFR-002 | ファイル書き込みはアトミック性を考慮（途中失敗でも既存ファイルを壊さない） |
| NFR-003 | `.git/info/exclude` のパーミッションを元のファイルと同等に維持する |

### 成功基準

| ID | 基準 | 検証方法 |
|----|------|----------|
| SC-001 | プロジェクトオープン後、全スキル/コマンド/フックファイルが正しい内容で配置される | ユニットテスト: 配置後のファイル内容検証 |
| SC-002 | `.git/info/exclude` にマネージドブロックが正しく書き込まれる | ユニットテスト: exclude ファイル内容検証 |
| SC-003 | 既存のユーザー独自 exclude ルールが保持される | ユニットテスト: 事前にカスタムルールを書き込み → 登録後にカスタムルールが残存確認 |
| SC-004 | マネージドブロック差し替えが冪等である（2 回実行で同じ結果） | ユニットテスト: 2 回実行 → diff なし確認 |
| SC-005 | worktree 環境で commondir の `.git/info/exclude` に書き込まれる | ユニットテスト: worktree セットアップ → exclude パス検証 |
| SC-006 | レガシーパターンが自動移行される | ユニットテスト: レガシーパターン入り exclude → 登録後にレガシー除去 + 新パターン確認 |
| SC-007 | 不正マーカー（入れ子 BEGIN、END なし）でエラーが返りファイルが変更されない | ユニットテスト: 不正パターン → エラー確認 → ファイル内容不変確認 |
| SC-008 | `git status` で gwt 管理ファイルが表示されない | 統合テスト: `git init` → 登録 → `git status` 確認 |
| SC-009 | `.claude/settings.local.json` にフック定義が登録される | ユニットテスト: settings.local.json の内容検証 |
| SC-010 | UNIX でスクリプトファイルに実行権限が付与される | ユニットテスト: パーミッション検証 |
| SC-011 | `.claude/settings.json` や `~/.claude/settings.json` が存在しても、登録結果が `.claude/settings.local.json` のみで決まること | ユニットテスト: legacy settings.json を置いても無視されることを検証 |
| SC-012 | repo ルート以外の CWD でも `.claude/settings.local.json` の hook コマンドが同じプロジェクトの script を解決できる | ユニットテスト: 生成コマンドに `git rev-parse --show-toplevel` が含まれることを検証 |
| SC-013 | Docker / DevContainer 内でも Git worktree が見えていれば同じ hook 定義で動作する | 手動確認: コンテナ内で `git rev-parse --show-toplevel` と hook 解決を確認 |

---
