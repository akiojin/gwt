# 実装計画: ログビューアのブランチ連動とエージェント出力取り込み

**仕様ID**: `SPEC-c1d5bad7` | **日付**: 2026-01-08 | **仕様書**: `specs/SPEC-c1d5bad7/spec.md`
**入力**: `specs/SPEC-c1d5bad7/spec.md`

## 概要

- ブランチ一覧で選択中のブランチに対応する worktree のログを表示する。
- ログ一覧に Branch/Source を表示し、当日が空の場合は同一ディレクトリ内の最新ログへフォールバックする。
- コーディングエージェント stdout/stderr をログとして取り込む（方法は要確認）。

## 技術コンテキスト

- **言語/ランタイム**: TypeScript 5.8 + Bun
- **CLI UI**: OpenTUI + SolidJS
- **ログ**: Pino JSONL (`~/.gwt/logs/<basename>/<YYYY-MM-DD>.jsonl`)
- **ターゲット**: CLI (Linux/macOS/Windows Terminal)
- **制約**: 既存のインタラクティブ TTY 体験を維持

## 原則チェック

- シンプルさ優先、既存実装の改修を優先
- TDD 必須（テスト→実装）
- 既存ファイル中心に改修し、新規ファイルは最小限

## プロジェクト構造

```text
specs/SPEC-c1d5bad7/
├── spec.md
├── plan.md
└── tasks.md

src/
├── cli/ui/
├── logging/
└── launcher.ts
```

## フェーズ0: 調査

- 既存のログ出力/読み込み経路と worktree 情報の取得フローを確認済み
- コーディングエージェントの stdout/stderr 取り込みは、TTY/PTY の扱いを再調査が必要

## フェーズ1: 設計

- **ログ対象ディレクトリ決定**: ブランチの worktree basename → `~/.gwt/logs/<basename>`
- **表示追加**: Log Viewer に Branch/Source を表示
- **stdout/stderr 取り込み**: PTY 経由でのストリームミラーリング案を検討

## 実装戦略

1. **P1**: ブランチ連動ログ表示と UI 表示追加
2. **P2**: エージェント stdout/stderr 取り込み（要確認事項を確定後）

## テスト戦略

- ログ対象ディレクトリ決定のユニットテスト
- Log Viewer の表示更新（Branch/Source, entries 更新）テスト
- stdout/stderr 取り込みが有効な場合のログ表示テスト

## リスクと緩和策

1. **TTY 互換性**: stdout/stderr 取り込みで対話型エージェントが崩れる
   - **緩和策**: 取り込みを opt-in にする、PTY 経由で画面出力を維持
2. **機密情報混入**: エージェント出力にシークレットが含まれる
   - **緩和策**: マスキングの適用範囲を明記し、必要なら opt-in にする

## 次のステップ

- `tasks.md` を更新し、TDD の実行順序を確定する
- **要確認事項**
  - 現在ブランチで worktree が無い場合のフォールバック扱い
  - stdout/stderr 取り込みの常時/opt-in 方針
