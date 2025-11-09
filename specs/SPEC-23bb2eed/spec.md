# 機能仕様: develop-to-main手動リリースフロー（Deprecated）

**仕様ID**: `SPEC-23bb2eed`
**作成日**: 2025-10-25
**最終更新日**: 2025-11-09
**ステータス**: 廃止（Superseded by `SPEC-57fde06f`）

本仕様は旧来の develop→main PR ベースのリリースフロー（`release-trigger.yml` など）を記述していましたが、2025-11 に release/vX.Y.Z ブランチを用いる unity-mcp-server 型フローへ完全移行しました。今後のリリース要件・設計・テストは **`specs/SPEC-57fde06f/`** を唯一の正とし、本ファイルは履歴参照のみとします。

- 最新のユーザーストーリー/FR/テスト: `specs/SPEC-57fde06f/spec.md`
- 実装計画/タスク: `specs/SPEC-57fde06f/plan.md`, `specs/SPEC-57fde06f/tasks.md`
- データモデル/Quickstart/Contracts: 同ディレクトリ配下を参照

このファイルに旧内容を残すと混乱を招くため削除しました。pull request 作成時は新仕様のみを基準にしてください。
