# 機能仕様: プロジェクトモード（Project Mode）

**仕様ID**: `SPEC-ba3f610c`
**作成日**: 2026-01-22
**更新日**: 2026-02-19
**ステータス**: 更新済み
**カテゴリ**: GUI
**入力**: 3層エージェントアーキテクチャ（Lead / Coordinator / Developer）によるプロジェクト統括。Lead（PM相当）がプロジェクトのゴールを保持し、要件定義・GitHub Issueベースの仕様管理・GitHub Project管理を担う。Coordinator（オーケストレーター）がDeveloper管理・CI監視・修正ループを行い、Developer（ワーカー）がWorktree内で実際の実装を行う。ユーザーはプロジェクト概要または具体的な機能要求のどちらでもLeadに伝えられる。
