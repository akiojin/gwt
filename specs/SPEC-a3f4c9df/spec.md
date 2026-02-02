# 機能仕様: 設定ファイル統合・整理

**仕様ID**: `SPEC-a3f4c9df`
**作成日**: 2026-02-02
**ステータス**: ドラフト
**入力**: ユーザー説明: "設定統合: 現在バラバラに保存されている9種類の設定ファイル（TOML/YAML/JSON混在、~/.config/gwt/と~/.gwt/の併存）を統一し、TOMLフォーマットと~/.gwt/ディレクトリに一元化する。段階的マイグレーションで後方互換性を維持しながら移行。"

## ユーザーシナリオとテスト *(必須)*

### ユーザーストーリー 1 - ProfilesのYAML→TOML移行 (優先度: P1)

開発者の既存profiles.yaml（YAML形式）がprofiles.toml（TOML形式）に自動マイグレーションされる。新形式が存在しない場合は旧形式から読み込み、保存時は新形式で書き込む。

**この優先度の理由**: Profilesは比較的シンプルな構造で、マイグレーションのリスクが低い。統合の基盤となるパターンを確立できる。

**独立したテスト**: profiles.yamlのみ存在する状態でgwtを起動し、保存後にprofiles.tomlが作成されることを確認することで検証できる。

**受け入れシナリオ**:

1. **前提条件** ~/.gwt/profiles.yamlのみ存在、**操作** gwtを起動してプロファイルを読み込む、**期待結果** 正常に読み込まれる
2. **前提条件** ~/.gwt/profiles.yamlのみ存在、**操作** プロファイルを保存、**期待結果** ~/.gwt/profiles.tomlが作成される
3. **前提条件** ~/.gwt/profiles.tomlと~/.gwt/profiles.yamlの両方が存在、**操作** 読み込み、**期待結果** profiles.toml（新形式）が優先される
4. **前提条件** profiles.yamlに無効なYAML、**操作** 読み込み、**期待結果** エラーログを出力し空のプロファイルリストを返す

---

### ユーザーストーリー 2 - ToolsのJSON→TOML移行 (優先度: P1)

開発者の既存tools.json（JSON形式）がtools.toml（TOML形式）に自動マイグレーションされる。グローバル（~/.gwt/）とローカル（.gwt/）の両方で対応。

**この優先度の理由**: カスタムエージェント機能の核心であり、ユーザーが頻繁に編集するファイル。

**独立したテスト**: tools.jsonのみ存在する状態でgwtを起動し、保存後にtools.tomlが作成されることを確認することで検証できる。

**受け入れシナリオ**:

1. **前提条件** ~/.gwt/tools.jsonのみ存在、**操作** カスタムエージェントを読み込む、**期待結果** 正常に読み込まれる
2. **前提条件** ~/.gwt/tools.jsonのみ存在、**操作** 保存、**期待結果** ~/.gwt/tools.tomlが作成される
3. **前提条件** .gwt/tools.jsonのみ存在、**操作** 保存、**期待結果** .gwt/tools.tomlが作成される
4. **前提条件** tools.tomlとtools.jsonの両方が存在、**操作** 読み込み、**期待結果** tools.toml（新形式）が優先される

---

### ユーザーストーリー 3 - BareProjectのJSON→TOML移行 (優先度: P1)

開発者の既存.gwt/project.json（JSON形式）が.gwt/project.toml（TOML形式）に自動マイグレーションされる。

**この優先度の理由**: ベアリポジトリ機能に必須。構造がシンプルでマイグレーションしやすい。

**独立したテスト**: project.jsonのみ存在する状態でgwtを起動し、保存後にproject.tomlが作成されることを確認することで検証できる。

**受け入れシナリオ**:

1. **前提条件** .gwt/project.jsonのみ存在、**操作** ベアプロジェクト設定を読み込む、**期待結果** 正常に読み込まれる
2. **前提条件** .gwt/project.jsonのみ存在、**操作** 保存、**期待結果** .gwt/project.tomlが作成される
3. **前提条件** project.tomlとproject.jsonの両方が存在、**操作** 読み込み、**期待結果** project.toml（新形式）が優先される

---

### ユーザーストーリー 4 - AgentHistoryのJSON→TOML移行 (優先度: P2)

開発者の既存agent-history.json（JSON形式）がagent-history.toml（TOML形式）に自動マイグレーションされる。

**この優先度の理由**: エージェント履歴は利便性機能であり、データ損失しても再生成可能。

**独立したテスト**: agent-history.jsonのみ存在する状態でgwtを起動し、保存後にagent-history.tomlが作成されることを確認することで検証できる。

**受け入れシナリオ**:

1. **前提条件** ~/.gwt/agent-history.jsonのみ存在、**操作** 履歴を読み込む、**期待結果** 正常に読み込まれる
2. **前提条件** ~/.gwt/agent-history.jsonのみ存在、**操作** 保存、**期待結果** ~/.gwt/agent-history.tomlが作成される
3. **前提条件** agent-history.tomlとagent-history.jsonの両方が存在、**操作** 読み込み、**期待結果** agent-history.toml（新形式）が優先される

---

### ユーザーストーリー 5 - グローバルディレクトリの統一 (優先度: P2)

グローバル設定ディレクトリを`~/.config/gwt/`から`~/.gwt/`に統一する。旧ディレクトリからの読み込みは維持しつつ、新規書き込みは`~/.gwt/`に行う。

**この優先度の理由**: ディレクトリ統一はユーザー体験の改善だが、既存機能への影響が大きい。

**独立したテスト**: ~/.config/gwt/にのみ設定が存在する状態でgwtを起動し、保存後に~/.gwt/に書き込まれることを確認することで検証できる。

**受け入れシナリオ**:

1. **前提条件** ~/.config/gwt/config.tomlのみ存在、**操作** 設定を読み込む、**期待結果** 正常に読み込まれる
2. **前提条件** ~/.gwt/config.tomlと~/.config/gwt/config.tomlの両方が存在、**操作** 読み込み、**期待結果** ~/.gwt/config.toml（新パス）が優先される
3. **前提条件** 初回起動、**操作** 設定を保存、**期待結果** ~/.gwt/config.tomlに書き込まれる

---

### ユーザーストーリー 6 - セッションファイルの統一 (優先度: P3)

TS互換セッション（~/.config/gwt/sessions/*.json）と新セッション（~/.gwt/sessions/*.toml）を統合し、TOML形式に一本化する。

**この優先度の理由**: セッションファイルは複雑な構造を持ち、マイグレーションのリスクが高い。

**独立したテスト**: 旧形式セッションのみ存在する状態でgwtを起動し、セッションが正常に読み込まれることを確認することで検証できる。

**受け入れシナリオ**:

1. **前提条件** ~/.config/gwt/sessions/*.jsonのみ存在、**操作** セッションを読み込む、**期待結果** 正常に読み込まれる
2. **前提条件** セッションを保存、**操作** 書き込み、**期待結果** ~/.gwt/sessions/*.tomlに書き込まれる
3. **前提条件** 新旧両方のセッションファイルが存在、**操作** 読み込み、**期待結果** TOML形式が優先される

---

### ユーザーストーリー 7 - 設定クリーンアップコマンド (優先度: P3)

開発者が`gwt config cleanup`コマンドを実行すると、マイグレーション済みの旧形式ファイルを削除できる。

**この優先度の理由**: クリーンアップは任意であり、マイグレーション完了後の補助機能。

**独立したテスト**: マイグレーション済みファイルが存在する状態で`gwt config cleanup`を実行し、旧ファイルが削除されることを確認することで検証できる。

**受け入れシナリオ**:

1. **前提条件** profiles.yamlとprofiles.tomlの両方が存在、**操作** `gwt config cleanup`を実行、**期待結果** profiles.yamlが削除される
2. **前提条件** クリーンアップ対象ファイルなし、**操作** `gwt config cleanup`を実行、**期待結果** "Nothing to clean up"メッセージ
3. **前提条件** 削除対象あり、**操作** `gwt config cleanup --dry-run`、**期待結果** 削除対象のリストのみ表示、実際の削除なし

---

### エッジケース

- 旧形式ファイルが破損している場合、どうなるか？→ エラーログを出力し、空のデフォルト値を使用。旧ファイルは.brokenに退避
- マイグレーション中にディスク容量不足で書き込み失敗した場合、どうなるか？→ 一時ファイルを削除し、旧形式のまま継続動作
- 旧形式と新形式で内容が異なる場合、どうなるか？→ 新形式を優先し、警告ログを出力
- ~/.gwt/ディレクトリが存在しない場合、どうなるか？→ 自動作成（パーミッション0700）
- シンボリックリンクの設定ファイルの場合、どうなるか？→ リンク先を読み込み、新形式は同じリンク先に書き込まない（新ファイル作成）

## 詳細仕様

### 統一後のディレクトリ構造

```
~/.gwt/                          # グローバル設定（統一）
├── config.toml                  # メイン設定
├── profiles.toml                # プロファイル (YAML→TOML)
├── tools.toml                   # カスタムエージェント (JSON→TOML)
├── agent-history.toml           # エージェント履歴 (JSON→TOML)
├── sessions/
│   └── {hash}.toml              # セッション（統一）
├── logs/{workspace}/            # ログ（変更なし）
└── conversions/                 # AI変換メタデータ（変更なし）

{project}/
├── .gwt.toml                    # ローカル設定（変更なし）
└── .gwt/
    ├── config.toml              # ローカル設定（変更なし）
    ├── tools.toml               # ローカルエージェント (JSON→TOML)
    └── project.toml             # ベアリポジトリ設定 (JSON→TOML)
```

### マイグレーション優先順位

読み込み時の優先順位（高い方が優先）:

1. 新形式（TOML）
2. 旧形式（YAML/JSON）

書き込みは常に新形式（TOML）に行う。

### マイグレーション対象ファイル一覧

| 旧ファイル | 新ファイル | フォーマット変換 |
|-----------|-----------|-----------------|
| ~/.gwt/profiles.yaml | ~/.gwt/profiles.toml | YAML → TOML |
| ~/.gwt/tools.json | ~/.gwt/tools.toml | JSON → TOML |
| .gwt/tools.json | .gwt/tools.toml | JSON → TOML |
| .gwt/project.json | .gwt/project.toml | JSON → TOML |
| ~/.gwt/agent-history.json | ~/.gwt/agent-history.toml | JSON → TOML |
| ~/.config/gwt/config.toml | ~/.gwt/config.toml | パス変更のみ |
| ~/.config/gwt/sessions/*.json | ~/.gwt/sessions/*.toml | JSON → TOML + パス変更 |

### 変更対象外

以下のファイルは変更しない:

- `~/.claude/*` - Claude Code管理のファイル
- `~/.gwt/logs/*` - ログファイル
- `~/.gwt/conversions/*` - AI変換メタデータ

### アトミック書き込み

データ安全性のため、以下の手順で書き込む:

1. 一時ファイル（`.tmp`拡張子）に書き込む
2. 一時ファイルをリネームして本ファイルに置き換える
3. 失敗時は一時ファイルを削除

### バックアップポリシー

- マイグレーション時、旧ファイルは自動削除しない
- `gwt config cleanup`で明示的に削除
- 破損ファイルは`.broken`拡張子で退避

### TOML形式の詳細

#### profiles.toml

```toml
version = "1.0.0"

[[profiles]]
name = "default"
description = "Default profile"

[profiles.env]
OPENAI_API_KEY = "sk-..."
```

#### tools.toml

```toml
version = "1.0.0"

[[custom_coding_agents]]
id = "aider"
display_name = "Aider"
type = "command"
command = "aider"
default_args = ["--no-git"]

[custom_coding_agents.mode_args]
normal = []
continue = ["--resume"]

[custom_coding_agents.env]
OPENAI_API_KEY = "sk-..."
```

#### project.toml

```toml
version = "1.0.0"
bare_repo_name = "my-project"
remote_url = "git@github.com:user/repo.git"
location = "/path/to/bare/repo"
```

## 要件 *(必須)*

### 機能要件

- **FR-001**: システムはprofiles.yaml（YAML形式）からprofiles.toml（TOML形式）へマイグレーションでき**なければならない**
- **FR-002**: システムはtools.json（JSON形式）からtools.toml（TOML形式）へマイグレーションでき**なければならない**
- **FR-003**: システムはproject.json（JSON形式）からproject.toml（TOML形式）へマイグレーションでき**なければならない**
- **FR-004**: システムはagent-history.json（JSON形式）からagent-history.toml（TOML形式）へマイグレーションでき**なければならない**
- **FR-005**: システムは新形式（TOML）が存在する場合、新形式を優先して読み込ま**なければならない**
- **FR-006**: システムは設定保存時、常に新形式（TOML）で書き込ま**なければならない**
- **FR-007**: システムは旧形式ファイルを自動削除**してはならない**
- **FR-008**: システムはアトミック書き込み（一時ファイル→リネーム）を使用**しなければならない**
- **FR-009**: システムは破損ファイルを`.broken`拡張子で退避**しなければならない**
- **FR-010**: システムは~/.gwt/ディレクトリが存在しない場合、パーミッション0700で自動作成**しなければならない**
- **FR-011**: システムは`gwt config cleanup`コマンドでマイグレーション済み旧ファイルを削除でき**なければならない**
- **FR-012**: システムは`gwt config cleanup --dry-run`で削除対象のプレビューを表示でき**なければならない**
- **FR-013**: システムはグローバル設定読み込み時、~/.gwt/を優先し、~/.config/gwt/をフォールバックとして使用**しなければならない**
- **FR-014**: システムはセッションファイルを~/.gwt/sessions/に統一**しなければならない**

### 主要エンティティ

- **MigrationManager**: マイグレーション処理を統括。旧形式検出、変換、書き込みを担当
- **ConfigPath**: 設定ファイルパスを管理。新旧パスの解決、優先順位判定を担当
- **FormatConverter**: YAML/JSON→TOML変換を担当。各設定構造の変換ロジックを持つ

## 成功基準 *(必須)*

### 測定可能な成果

- **SC-001**: 旧形式のみ存在する環境でgwtが正常に起動し、設定を読み込める
- **SC-002**: 保存後、新形式ファイルが作成される
- **SC-003**: 新旧両形式が存在する場合、新形式が優先される
- **SC-004**: マイグレーションでデータ損失が発生しない
- **SC-005**: `gwt config cleanup`で旧ファイルが正常に削除される

## 制約と仮定 *(該当する場合)*

### 制約

- Claude Code管理ファイル（~/.claude/*）は変更しない
- 既存の設定読み込み優先順位（ローカル > グローバル）は維持
- マイグレーションは自動実行、旧ファイル削除は明示的コマンド

### 仮定

- ユーザーは~/.gwt/ディレクトリへの書き込み権限を持つ
- 設定ファイルは手動編集される可能性がある（TOML形式の可読性が重要）
- 既存の旧形式ファイルは有効な形式で保存されている

## 範囲外 *(必須)*

次の項目は、この機能の範囲外です：

- Claude Code管理ファイルの変更
- ログファイルやAI変換メタデータのマイグレーション
- 設定ファイルのGUIエディタ
- 複数バージョン間のスキーママイグレーション（v1.0.0のみ対応）
- 設定ファイルのバリデーションエラーの自動修復

## セキュリティとプライバシーの考慮事項 *(該当する場合)*

- 設定ファイルにはAPIキーが含まれる可能性があるため、パーミッション0600を推奨
- ディレクトリパーミッションは0700
- 一時ファイルも同じパーミッションで作成
- バックアップファイル（.broken）も同じパーミッションを維持

## 依存関係 *(該当する場合)*

- 既存のSettings機能（settings.rs）
- 既存のProfile機能（profile.rs）
- 既存のTools機能（tools.rs）
- 既存のSession機能（session.rs, ts_session.rs）
- 既存のBareProject機能（bare_project.rs）
- tomlクレート（シリアライゼーション）
- serde_yamlクレート（YAMLパース）

## 参考資料 *(該当する場合)*

- [TOML v1.0.0仕様](https://toml.io/ja/v1.0.0)
- [既存Profile仕様](../SPEC-dafff079/spec.md)
- [既存Tools仕様](../SPEC-71f2742d/spec.md)
