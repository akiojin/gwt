### 背景

仕様管理をローカル `specs/SPEC-[hex8]/` から GitHub Issue に完全移行する。

**現状**:
- Rust バックエンド（`issue_spec.rs`）に Issue body CRUD 実装済み
- GUI（IssueSpecPanel）は Issue 番号ベースで動作済み
- Spec Kit（`.specify/` + `.claude/commands/speckit.*.md`）がローカルファイル専用のまま残存

**ゴール**:
- GitHub Issue が仕様の Single Source of Truth
- Spec Kit を完全廃止し、スキル（`gwt-issue-spec-ops`）+ Rust/Tauri API に一本化
- 既存 specs を Issue にバッチ移行
- GitHub Project (v2) でライフサイクル管理

### 設計判断

1. **SPEC ID = Issue 番号** — `SPEC-[a-f0-9]{8}` を廃止
2. **Spec Kit を完全廃止** — `.specify/` ディレクトリと `speckit.*` コマンドを全削除
3. **gwt-issue-spec-ops スキル** がエージェントの操作インターフェース
4. **Rust/Tauri API** がユーザー（GUI）の操作インターフェース
5. **通常 Issue との統合** — Spec Issue は `gwt-spec` ラベル付きの通常 Issue

### ユーザーシナリオ

- **US1**: エージェントが仕様を作成 → GitHub Issue（gwt-spec ラベル）として作成される
- **US2**: GUI で Spec Issue を閲覧 → IssueSpecPanel で表示される
- **US3**: 既存 specs/ の仕様を参照 → GitHub Issue から参照可能
- **US4**: プロジェクトでライフサイクル管理 → GitHub Project で Phase 管理

### 成功基準

- [ ] GitHub Issue が仕様の Single Source of Truth
- [ ] `gwt-spec` ラベルでフロントエンド/バックエンドが統一
- [ ] Spec Kit 関連ファイルが全て削除
- [ ] 既存 specs が Issue に移行済み
- [ ] CLAUDE.md が新ワークフローを反映
