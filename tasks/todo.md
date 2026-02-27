# TODO: Project Mode 改善 — 設計ドキュメント更新

## 背景

Project Mode の仕様（SPEC-ba3f610c）を新ビジョンに基づいて更新する。設計のみ、コード変更なし。

主な変更:

- Developer → Worker リネーム（3層目の名称変更）
- Lead の役割再定義（プロジェクト管理者 + 要件収集フロー）
- Worker ペルソナ/プリセットシステムの設計
- GUI ペルソナ設定画面の設計
- チャット UI 強化の設計

## 実装ステップ

- [x] spec.md の仕様更新（Developer→Worker、Lead再定義、ペルソナ追加、GUI設計）
- [x] data-model.md の更新（Worker/Persona型追加、LeadStatus更新）
- [x] plan.md の実装計画更新（ペルソナフェーズ追加、Lead拡張フェーズ追加）
- [x] tasks.md のタスク一覧更新（ペルソナ・Lead拡張タスク追加）
- [x] tdd.md / quickstart.md の整合性更新（Developer→Worker）
- [x] 6ファイル間の整合性確認（markdownlint通過）

## 検証結果

- [x] markdownlint: 全6ファイルでエラー・警告なし
- [x] Developer残留確認: spec.md 2件（意図的: ペルソナ名 + 変更前対比）、research.md 3件（更新対象外）のみ
- [x] 新規タスク追加確認: T106, T205-T214, T315-T316, T511-T512, T1217-T1220（計19件）
