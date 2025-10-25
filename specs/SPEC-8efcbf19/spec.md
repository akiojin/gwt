# 機能仕様: Docker/root環境でのClaude Code自動承認機能

**仕様ID**: `SPEC-8efcbf19`
**作成日**: 2025-10-25
**ステータス**: ドラフト
**入力**: ユーザー説明: "Docker/root環境でClaude Codeの--dangerously-skip-permissionsを動作させるため、IS_SANDBOX=1環境変数を自動設定する機能を追加する。rootユーザー検出時に環境変数を設定し、適切な警告メッセージを表示する。"

## ユーザーシナリオとテスト *(必須)*

### ユーザーストーリー 1 - Docker/root環境での自動承認実行 (優先度: P1)

開発者として、Docker環境でrootユーザーとして実行している際に、Claude Codeの`--dangerously-skip-permissions`フラグを使用してPermission promptなしでClaude Codeを実行したい。

**この優先度の理由**: Docker環境はサンドボックス化されており、rootユーザーでの実行が一般的です。現在、Claude CodeはセキュリティルールによりrootユーザーでのPermission skip機能を拒否していますが、コンテナ環境では安全に使用できるため、この制限を回避する必要があります。

**独立したテスト**: Docker環境でrootユーザーとしてClaude Codeを起動し、`--dangerously-skip-permissions`フラグを使用してPermission promptなしで動作することを確認できます。

**受け入れシナリオ**:

1. **前提条件** Docker環境でrootユーザーとして実行、**操作** Claude Worktreeから新規worktreeを作成しClaude Codeを起動（skipPermissions=true）、**期待結果** IS_SANDBOX=1環境変数が自動的に設定され、Claude Codeがエラーなく起動し、Permission promptが表示されない
2. **前提条件** Docker環境でrootユーザーとして実行、**操作** 既存セッションからClaude Codeを継続実行（skipPermissions=true）、**期待結果** IS_SANDBOX=1環境変数が設定され、エラーなく実行される
3. **前提条件** Docker環境でrootユーザーとして実行、**操作** skipPermissions=falseでClaude Codeを起動、**期待結果** IS_SANDBOX=1環境変数は設定されず、通常のPermission prompt動作となる

---

### ユーザーストーリー 2 - セキュリティ警告の表示 (優先度: P2)

開発者として、rootユーザーでPermission skipを使用する際に、セキュリティリスクについての警告メッセージを確認したい。

**この優先度の理由**: ユーザーがrootユーザーでPermission skipを使用していることを認識し、セキュリティリスクを理解することが重要です。警告メッセージにより、意図しない誤用を防ぎます。

**独立したテスト**: rootユーザーでskipPermissions=trueで起動した際に、適切な警告メッセージが表示されることを確認できます。

**受け入れシナリオ**:

1. **前提条件** Docker環境でrootユーザーとして実行、**操作** Claude CodeをskipPermissions=trueで起動、**期待結果** "Docker/サンドボックス環境として実行中（IS_SANDBOX=1）"という警告メッセージが表示される
2. **前提条件** Docker環境でrootユーザーとして実行、**操作** Claude CodeをskipPermissions=trueで起動、**期待結果** "Skipping permissions check"という警告メッセージが表示される

---

### ユーザーストーリー 3 - 非rootユーザーでの既存動作維持 (優先度: P3)

開発者として、非rootユーザーでClaude Codeを実行する際に、既存の動作が変更されないことを確認したい。

**この優先度の理由**: 非rootユーザーでの実行は既に公式にサポートされており、この機能追加により既存の動作が影響を受けないことを保証する必要があります。

**独立したテスト**: 非rootユーザーでClaude Codeを起動し、既存の動作と同じであることを確認できます。

**受け入れシナリオ**:

1. **前提条件** 非rootユーザーとして実行、**操作** Claude CodeをskipPermissions=trueで起動、**期待結果** IS_SANDBOX=1環境変数は設定されず、既存の--dangerously-skip-permissions動作となる
2. **前提条件** 非rootユーザーとして実行、**操作** Claude CodeをskipPermissions=falseで起動、**期待結果** 通常のPermission prompt動作となり、rootユーザー検出の警告は表示されない

---

### エッジケース

- rootユーザー検出が失敗した場合（例: process.getuid()が利用できない環境）、どのように動作しますか？
- IS_SANDBOX=1環境変数がClaude Codeの将来のバージョンで動作しなくなった場合、どのようにユーザーに通知しますか？
- skipPermissions=trueかつroot環境で、IS_SANDBOX=1を設定してもClaude Codeがエラーを返した場合、どのようにハンドリングしますか？

## 要件 *(必須)*

### 機能要件

- **FR-001**: システムは実行中のユーザーがrootユーザーかどうかを検出**しなければならない**（process.getuid() === 0を使用）
- **FR-002**: rootユーザーで実行中かつskipPermissions=trueの場合、システムはIS_SANDBOX=1環境変数を設定**しなければならない**
- **FR-003**: rootユーザーで実行中かつskipPermissions=trueの場合、システムは"Docker/サンドボックス環境として実行中（IS_SANDBOX=1）"という警告メッセージを表示**しなければならない**
- **FR-004**: 非rootユーザーで実行中の場合、システムはIS_SANDBOX=1環境変数を設定せず、既存の動作を維持**しなければならない**
- **FR-005**: rootユーザー検出が失敗した場合（process.getuid()が利用できない環境）、システムは既存の動作を維持**しなければならない**

### 主要エンティティ

- **環境変数設定**: Claude Code起動時に設定される環境変数（IS_SANDBOX=1）。rootユーザー検出とskipPermissionsフラグに基づいて条件付きで設定される。
- **警告メッセージ**: rootユーザーでPermission skip使用時に表示されるメッセージ。セキュリティリスクをユーザーに通知する。

## 成功基準 *(必須)*

### 測定可能な成果

- **SC-001**: Docker環境でrootユーザーとしてClaude Codeを`skipPermissions=true`で起動した際、エラーなく起動しPermission promptが表示されないこと
- **SC-002**: rootユーザーでskipPermissions=trueで起動した際、"Docker/サンドボックス環境として実行中"という警告メッセージがコンソールに表示されること
- **SC-003**: 非rootユーザーでClaude Codeを起動した際、既存の動作と同じであり、IS_SANDBOX=1環境変数が設定されないこと
- **SC-004**: rootユーザー検出が失敗した環境（process.getuid()が利用できない）でも、システムがエラーなく動作すること

## 制約と仮定 *(該当する場合)*

### 制約

- IS_SANDBOX=1環境変数は非公式の環境変数であり、Anthropic社による公式ドキュメントが存在しない
- Claude Codeの将来のバージョンでIS_SANDBOX=1環境変数が動作しなくなる可能性がある
- process.getuid()はPOSIX準拠のシステム（Linux、macOS）でのみ利用可能であり、Windows環境では動作しない

### 仮定

- Docker/コンテナ環境での使用を前提としており、セキュリティリスクを理解したユーザーが使用する
- Claude Code CLIはIS_SANDBOX=1環境変数を検出し、rootユーザーでの--dangerously-skip-permissions使用を許可する
- ユーザーはCLAUDE.mdに記載されている開発指針（セキュリティリスクの理解、Docker環境での使用）を理解している

## 範囲外 *(必須)*

次の項目は、この機能の範囲外です：

- IS_SANDBOX=1環境変数の公式サポートの提供（これはClaude Code側の責任）
- 非Docker環境（ローカルホストのroot実行など）での推奨使用
- rootユーザー以外のPermission skip機能の改善
- Windows環境でのrootユーザー検出の実装

## セキュリティとプライバシーの考慮事項 *(該当する場合)*

- **セキュリティリスク**: rootユーザーでPermission skipを使用することは、システムに対する完全なアクセスを許可することを意味します。Docker/コンテナ環境以外での使用は非推奨です。
- **警告メッセージ**: rootユーザー実行時に警告メッセージを表示することで、ユーザーがセキュリティリスクを認識できるようにします。
- **環境制限**: IS_SANDBOX=1環境変数の設定は、明示的にskipPermissions=trueが指定された場合のみ行われ、ユーザーの意図しない自動設定を防ぎます。

## 依存関係 *(該当する場合)*

- Node.js process.getuid() API（POSIXシステムでのみ利用可能）
- execa環境変数設定機能
- Claude Code CLI（@anthropic-ai/claude-code）のIS_SANDBOX=1環境変数サポート
- 既存のlaunchClaudeCode関数（src/claude.ts）

## 参考資料 *(該当する場合)*

- [Claude Code GitHub Issue #3490 - Root permission discussion](https://github.com/anthropics/claude-code/issues/3490)
- [Community discovery: IS_SANDBOX=1 environment variable](https://github.com/anthropics/claude-code/issues/3490#issuecomment)
- [SPEC-c0deba7e: AIツール(Claude Code / Codex CLI)のbunx移行](../SPEC-c0deba7e/spec.md)
