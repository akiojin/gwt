# 機能仕様: Claude Code プラグインマーケットプレイス自動登録

**仕様ID**: `SPEC-f8dab6e2`
**作成日**: 2026-01-30
**ステータス**: ドラフト
**入力**: ユーザー説明: "gwtを起動した場合にworktree-protection-hooksプラグインのマーケットプレイス登録とプラグイン有効化を自動設定する"

## ユーザーシナリオとテスト *(必須)*

### ユーザーストーリー 1 - マーケットプレイス未登録時の自動登録 (優先度: P1)

ユーザーがgwtでClaude Codeを起動しようとした時、gwt-pluginsマーケットプレイスが未登録であれば、確認ダイアログを表示し、ユーザーの同意を得てマーケットプレイスとプラグインを自動登録する。

**この優先度の理由**: これがコア機能であり、worktree-protection-hooksプラグインを利用可能にするために必須。

**独立したテスト**: 空の`known_marketplaces.json`でgwtからClaude Codeを起動し、確認ダイアログが表示され、同意後に登録されることを確認。

**受け入れシナリオ**:

1. **前提条件** `known_marketplaces.json`に`gwt-plugins`が未登録、**操作** gwtでClaude Codeを起動、**期待結果** 確認ダイアログが表示される
2. **前提条件** 確認ダイアログが表示されている、**操作** 「Setup」を選択、**期待結果** `known_marketplaces.json`に`gwt-plugins`が追加され、`enabledPlugins`に`worktree-protection-hooks@gwt-plugins`が追加される
3. **前提条件** 確認ダイアログが表示されている、**操作** 「Skip」を選択、**期待結果** 何も登録されずClaude Codeが起動される

---

### ユーザーストーリー 2 - マーケットプレイス登録済み時のスキップ (優先度: P1)

ユーザーがgwtでClaude Codeを起動しようとした時、gwt-pluginsマーケットプレイスが既に登録されていれば、確認ダイアログを表示せずにClaude Codeを起動する。

**この優先度の理由**: 登録済みユーザーの操作を妨げないために必須。

**独立したテスト**: `known_marketplaces.json`に`gwt-plugins`が登録済みの状態でgwtからClaude Codeを起動し、確認ダイアログが表示されないことを確認。

**受け入れシナリオ**:

1. **前提条件** `known_marketplaces.json`に`gwt-plugins`が登録済み、**操作** gwtでClaude Codeを起動、**期待結果** 確認ダイアログが表示されずClaude Codeが起動される

---

### ユーザーストーリー 3 - Codex選択時は登録をスキップ (優先度: P2)

ユーザーがgwtでCodexを起動しようとした時、マーケットプレイス登録の確認ダイアログは表示しない（Claude Code専用機能のため）。

**この優先度の理由**: Codexユーザーに不要な確認を表示しないために重要。

**独立したテスト**: エージェント選択でCodexを選択し、確認ダイアログが表示されないことを確認。

**受け入れシナリオ**:

1. **前提条件** `known_marketplaces.json`に`gwt-plugins`が未登録、**操作** gwtでCodexを起動、**期待結果** 確認ダイアログが表示されずCodexが起動される

---

### ユーザーストーリー 4 - ファイル/ディレクトリの自動作成 (優先度: P2)

`~/.claude/plugins/`ディレクトリや`known_marketplaces.json`が存在しない場合、自動的に作成する。

**この優先度の理由**: 新規ユーザーでもスムーズに設定できるために重要。

**独立したテスト**: `~/.claude/plugins/`が存在しない状態で登録を実行し、ディレクトリとファイルが作成されることを確認。

**受け入れシナリオ**:

1. **前提条件** `~/.claude/plugins/`が存在しない、**操作** 確認ダイアログで「Setup」を選択、**期待結果** `~/.claude/plugins/`ディレクトリと`known_marketplaces.json`が作成される

---

### エッジケース

- `known_marketplaces.json`が不正なJSON形式の場合、何が起こりますか？ → サイレントに新規ファイルとして扱う
- ファイル書き込み権限がない場合、何が起こりますか？ → サイレントに続行（Claude Codeは起動される）
- ユーザーがプラグインを手動で無効化した後に再起動した場合、何が起こりますか？ → 再有効化しない（ユーザー設定を尊重）

## 要件 *(必須)*

### 機能要件

- **FR-001**: システムはClaude Code起動時に`~/.claude/plugins/known_marketplaces.json`の`gwt-plugins`登録状態を確認**しなければならない**
- **FR-001b**: 登録済み判定は`installLocation`と`lastUpdated`が文字列として存在することを含む
- **FR-002**: システムは未登録時に確認ダイアログを表示**しなければならない**
- **FR-003**: システムはユーザーが「Setup」を選択した場合、`known_marketplaces.json`に以下の形式で登録**しなければならない**:
  ```json
  "gwt-plugins": {
    "source": {"source": "github", "repo": "akiojin/gwt"},
    "installLocation": "<string>",
    "lastUpdated": "<string>"
  }
  ```
- **FR-003b**: `installLocation`と`lastUpdated`は空文字ではない文字列でなければならない
- **FR-004**: システムはユーザーが「Setup」を選択した場合、`~/.claude/settings.json`と`.claude/settings.json`の`enabledPlugins`に`worktree-protection-hooks@gwt-plugins: true`を追加**しなければならない**
- **FR-005**: システムは`.claude/`ディレクトリが存在しない場合、作成**しなければならない**
- **FR-006**: システムは`~/.claude/plugins/`ディレクトリが存在しない場合、作成**しなければならない**
- **FR-007**: システムはCodex起動時には確認ダイアログを表示**してはならない**
- **FR-008**: システムは登録済みの場合、確認ダイアログを表示**してはならない**
- **FR-009**: システムはファイル書き込みエラー時、サイレントに続行**しなければならない**
- **FR-010**: システムはユーザーが手動で無効化したプラグインを再有効化**してはならない**

### 主要エンティティ

- **KnownMarketplaces**: `~/.claude/plugins/known_marketplaces.json`に保存されるマーケットプレイス登録情報
- **EnabledPlugins**: `settings.json`の`enabledPlugins`セクションに保存されるプラグイン有効化情報

## 成功基準 *(必須)*

### 測定可能な成果

- **SC-001**: Claude Code起動時、未登録の場合は確認ダイアログが表示される
- **SC-002**: 「Setup」選択後、`known_marketplaces.json`に正しい形式で登録される
- **SC-003**: 「Setup」選択後、`enabledPlugins`に正しいエントリが追加される
- **SC-004**: 登録済みの場合、確認ダイアログなしでClaude Codeが起動される
- **SC-005**: Codex選択時は確認ダイアログが表示されない

## 制約と仮定 *(該当する場合)*

### 制約

- 既存の`gwt hook setup`ダイアログとは別のダイアログとして実装する（Codexユーザーには不要なため）
- Claude Codeのプラグインシステムの仕様に依存する

### 仮定

- Claude Codeは`known_marketplaces.json`を読み込み、`enabledPlugins`に設定されたプラグインを自動的にインストールする
- `~/.claude/plugins/`ディレクトリはClaude Codeが使用する標準的なプラグイン管理ディレクトリである
- `source.source: "github"`と`source.repo`の形式でGitHubリポジトリからマーケットプレイスを解決できる

## 範囲外 *(必須)*

次の項目は、この機能の範囲外です：

- プラグインの実際のインストール（Claude Codeが行う）
- Codexのプラグインシステムサポート（将来の拡張として検討）
- マーケットプレイスの更新機能
- プラグインの削除・無効化機能

## セキュリティとプライバシーの考慮事項 *(該当する場合)*

- ファイル書き込みは`~/.claude/`配下のみに限定する
- 外部通信は行わない（マーケットプレイス情報の登録のみ）

## 依存関係 *(該当する場合)*

- `~/.claude/plugins/known_marketplaces.json`のファイル形式
- `settings.json`の`enabledPlugins`セクションの形式
- Claude Codeのプラグイン解決メカニズム

## 参考資料 *(該当する場合)*

- [Claude Code プラグインドキュメント](https://docs.anthropic.com/claude-code/plugins)
- [既存のgwt hook setup実装](../SPEC-861d8cdf/spec.md)
