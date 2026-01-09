# 調査: 画面表示時刻のシステムロケール変換

- 調査日: 2026-01-09
- 対象: CLI (OpenTUI/SolidJS) と Web UI (React)

## 既存の表示フォーマッタ

- CLI: `src/cli/ui/utils/branchFormatter.ts`（年月日+時刻）
- CLI: `src/logging/formatter.ts`（時刻のみ）
- CLI: `src/cli/ui/utils/versionFetcher.ts`（日付のみ）
- Web UI: `src/web/client/src/pages/BranchDetailPage.tsx`（日時）
- Web UI: `src/web/client/src/components/branch-detail/SessionHistoryTable.tsx`（日時）
- Web UI: `src/web/client/src/components/EnvEditor.tsx`（更新日時）

## UTC表示が起きうる箇所

- ISO8601文字列を画面に直接出力している箇所
- 保存/送信用の `toISOString` を表示用途に流用している箇所
- 並べ替え用の文字列を表示に転用している箇所

## 結論

- 既存フォーマット関数を中心にローカル表示へ統一する
- 表示に関わる箇所を監査し、UTC文字列の直出しを排除する
