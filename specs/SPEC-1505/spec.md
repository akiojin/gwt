> **ℹ️ TUI MIGRATION NOTE**: This SPEC was completed during the gwt-tauri era. The gwt-tauri frontend has been replaced by gwt-tui (SPEC-1776). GUI-specific references are historical.

# バグ修正仕様: PR 作成前の branch preflight を Codex / Claude Code / GWT で必須化

**作成日**: 2026-03-06
**更新日**: 2026-03-06
**ステータス**: ドラフト
**カテゴリ**: Workflow / GUI / Skills
**依存仕様**:

- #1359（PRタブへのWorkflow統合とブランチ状態表示）
- #1368（PR Dashboard）
- #1438（スキル埋め込み — 登録/解除ライフサイクル）

**入力**: ユーザー説明: 「PR 作成前に base との差分を必ず点検し、behind のまま PR を作成させない。gwt 埋め込みだけでなく、ユーザーホームの gh-pr と Claude Code 側も同様に扱う」
