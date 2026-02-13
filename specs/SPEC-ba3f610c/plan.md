# 実装計画: エージェントモード（GUI版）

**仕様ID**: `SPEC-ba3f610c`  
**日付**: 2026-02-13  
**仕様書**: [spec.md](./spec.md)

## 概要

マスターエージェント（MA）との対話を中心に、Agentビューでタスクと担当サブエージェントを可視化する。  
本計画では、チャット表示改善に加えて以下を同期実装する。

- Agentビューのタスク一覧表示（状態付き）
- タスク選択時の担当サブエージェント一覧表示
- worktree相対パス表示と詳細時の絶対パス表示
- 再計画時の即時同期（現在担当のみ表示）
- MAによるSpec Kit成果物（`spec.md`/`plan.md`/`tasks.md`/`tdd.md`）の生成完了チェック

## 技術コンテキスト

- 言語: Rust 2021 Edition / TypeScript
- GUI: Tauri v2 + Svelte 5 + xterm.js
- バックエンド: gwt-tauri + gwt-core
- ストレージ: ファイルシステム（`~/.gwt/sessions/`）
- テスト: cargo test / pnpm test / vitest

## 実装スコープ

1. MAセッション状態モデルの拡張（Task/SubAgent/worktree表示情報）
2. Agentビューのタスク一覧UI実装（`running > pending > failed > completed`）
3. タスク選択時の担当サブエージェント一覧UI実装（全件表示）
4. 複数サブエージェント割当の表示対応
5. 再計画・再割当時の同期表示（現在担当のみ）
6. worktree表示仕様（相対デフォルト + ホバー/詳細で絶対）
7. Spec Kit成果物4点（`spec.md`/`plan.md`/`tasks.md`/`tdd.md`）生成と実行ゲート制御
8. 既存チャット要件（IME、スピナー、自動スクロール）の回帰防止

## 主要コード構成

```text
crates/gwt-tauri/src/
├── agent_master.rs                # MA状態・応答ループ
├── state.rs                       # ウィンドウごとのAgent Mode状態
├── commands/agent_mode.rs         # Agent Mode用Tauriコマンド
└── session/* (必要に応じて)       # セッション永続化

gwt-gui/src/
├── lib/components/AgentModePanel.svelte
├── lib/components/AgentSidebar.svelte
└── lib/types.ts
```

## 実装方針

- UIは「チャット」と「Agentビュー」を同一セッション状態で駆動する。
- タスク選択を単一選択に統一し、下部一覧は選択タスクに限定して描画する。
- サブエージェント一覧は全件表示し、再計画時は現在担当だけを保持する。
- worktreeは表示用相対パスと詳細用絶対パスを同時保持する。
- 実行フェーズ遷移前に成果物4点の存在検証を行い、不足時はMAが再生成またはユーザー確認を行う。

## 受け入れ条件

- MAチャットでユーザー入力を受け付け、既存UI要件（IME/スピナー/自動スクロール）を満たす
- Agentビューにタスク一覧が状態付きで表示される
- タスク一覧の表示順が `running > pending > failed > completed` になる
- タスク選択時に下部へ担当サブエージェント一覧が全件表示される
- 1タスクに複数担当がある場合も全件表示される
- 再計画時、下部一覧は現在担当のみを表示し過去担当を表示しない
- worktreeは相対パスを表示し、ホバーまたは詳細で絶対パスを確認できる
- MAは`spec.md`/`plan.md`/`tasks.md`/`tdd.md`が揃うまで実行を開始しない

## 検証方針

- フロントエンド: Agentビュー表示、選択連動、並び順、worktree表示形式をコンポーネントテストで検証
- バックエンド: MA状態変換、再割当時の現在担当のみ保持、成果物4点の実行ゲートをユニットテストで検証
- 回帰: 既存 `AgentModePanel` のIME/スピナー/オートスクロールテストを維持
