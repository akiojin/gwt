# Spec Kit 設定

- ベースバージョン: spec-kit v0.0.94
- 最終更新: 2026-02-12
- ローカル運用ルール:
  - 日本語で運用する（テンプレート/コマンド/ガイドは日本語）
  - ブランチ操作は禁止（スクリプトで branch 作成/切替をしない）
  - 仕様IDは `SPEC-[a-f0-9]{8}` を使用

## 構成

- `.specify/templates/`: 仕様/計画/タスク/憲章などのテンプレート
- `.specify/scripts/`: Spec Kit コマンドが参照する補助スクリプト
- `.specify/memory/`: 憲章（constitution）などの共有メモリ
