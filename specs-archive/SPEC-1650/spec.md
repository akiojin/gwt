> **ℹ️ TUI MIGRATION NOTE**: This SPEC describes backend/gwt-core functionality unaffected by the gwt-tui migration (SPEC-1776). No changes required.
> **Status Note**: この SPEC は実装完了により closed。files semantic search 基盤の参照として保持し、追加変更は新規 child SPEC で管理する。

# プロジェクトファイルインデックス

## Background
プロジェクトファイルのベクトル検索機能を提供する。Studio時代の #1554（プロジェクトインデックス＆検索）の機能概念を現行スタックで再定義。

GitHub Issue/PR のインデックス機能は #1684 (GitHub Issue/PR Index) に分離。
Assistant Mode連携（変更とIssueの関連付け）は #1636 FR-10 でカバー済み。

## User Stories

**S1: ファイル内容のセマンティック検索**
- Given: プロジェクトファイルがインデックス済み
- When: 自然言語で検索する
- Then: 関連するファイル・箇所が返される

**S2: インデックス更新**
- Given: ファイルが変更される
- When: インデックス更新がトリガーされる
- Then: インデックスが最新状態に更新される

## Functional Requirements

**FR-01: ベクトル検索**
- ファイル内容のベクトル化
- セマンティック検索

**FR-02: インデックス管理**
- インクリメンタル更新
- インデックスの永続化

**FR-03: スキル責務分離**
- ファイル検索スキル（gwt-project-search）とIssue検索スキル（gwt-issue-search）を分離
- 4つの呼び出し側スキルはIssue検索のみ使用しており、ファイル検索の不要なコンテキスト注入を排除
- バックエンド（Rust/Python/GUI）は変更不要。スキル/コマンドMarkdownとRust登録コードのみ変更

## Success Criteria

1. セマンティック検索が正確な結果を返す
2. インデックスが効率的に更新される

---
