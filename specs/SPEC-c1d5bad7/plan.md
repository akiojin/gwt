# 実装計画: ログビューアのブランチ連動とエージェント出力取り込み

**仕様ID**: `SPEC-c1d5bad7` | **日付**: 2026-01-08 | **仕様書**: `specs/SPEC-c1d5bad7/spec.md`
**入力**: `specs/SPEC-c1d5bad7/spec.md`

## 概要

- ブランチ一覧で選択中のブランチに対応する worktree のログを表示する。
- ログ一覧に Branch/Source を表示し、当日が空の場合は同一ディレクトリ内の最新ログへフォールバックする。
- コーディングエージェント stdout/stderr を **opt-in** でログとして取り込む（`GWT_CAPTURE_AGENT_OUTPUT=true|1`）。

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
- stdout/stderr 取り込みは PTY 経由でミラーリングし、TTY体験を維持する方針

## フェーズ1: 設計

- **ログ対象ディレクトリ決定**: ブランチの worktree basename → `~/.gwt/logs/<basename>`（現在ブランチの worktree 無しは起動ディレクトリをフォールバック）
- **表示追加**: Log Viewer に Branch/Source を表示
- **stdout/stderr 取り込み**: PTY 経由でストリームをミラーリングし、ログへ追記

## 実装戦略

1. **P1**: ブランチ連動ログ表示と UI 表示追加
2. **P2**: エージェント stdout/stderr 取り込み（opt-in）

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
