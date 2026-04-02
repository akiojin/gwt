# Research: SPEC-1776 — Cross-Spec Comparison Matrix

## Purpose

`SPEC-1776` は単独で全機能を上書きしない。親 SPEC として、旧TUI、現行 `gwt-tui`、現行 `gwt-core`、関連 child SPEC を横断して比較し、新しい TUI の優先順位だけを定義する。

## Comparison Matrix

| Concern | 旧TUI (`v6.30.3`) | 現行 `gwt-tui` | 現行 `gwt-core` | 新目標 |
|---|---|---|---|---|
| Primary entry | `branch list` 中心 | session/tab surface が強い | Git / launch data は再利用可能 | `Branches` を primary entry に戻す |
| Session multiplicity | 基本 `1ブランチ = 1セッション` | 同一ブランチ複数セッション可 | session persistence あり | `1ブランチ = Nセッション` を正式化 |
| Session layout | `tmux` pane / focus | tab + single-pane寄り | `native PTY` / scrollback あり | `equal grid` + `maximize` |
| Session visibility from branch rows | 行内で起動中 agent が見える | 情報はあるが UX が散る | branch/session linkage は利用可 | 各行は `session count` のみに絞る |
| Enter behavior | focus / show hidden / wizard | tab 前提 | launch primitives あり | `no / one / many` の 3 分岐 |
| Hidden pane concept | あり | なし | 不要 | 廃止 |
| Management workspace | 旧TUIは tab ではない | tabbed management | data loaders あり | tabbed management を採用 |
| Initial management tabs | 旧TUIは統合的 | Branches / SPECs / Issues / Versions / Settings / Logs | backend support は広い | 初回は `Branches / SPECs / Issues / Profiles` |
| Terminal basis | `tmux` + terminal | `native PTY` + `vt100` | `terminal::*` 完備 | 現行 terminal 基盤を維持 |
| Quick Start | あり | child 実装あり | session store / detect あり | Branches flow に接続維持 |
| SPECs tab | 旧TUIには直接対応薄い | local artifact viewer あり | local SPEC loader あり | tab 維持、parent navigation に同期 |
| Issues tab | GitHub Issue 導線あり | GitHub Issue detail あり | issue cache / linkage あり | tab 維持、launch entry 維持 |
| Profiles | Settings 内の env 管理 | Settings 内に存在 | persistence あり | `Profiles = Env profiles` として独立タブ化 |
| Settings | 強い設定面あり | 実装済み | persistence あり | 後続 |
| Logs | 旧TUIに存在 | 実装済み | log reader あり | 後続 |
| Versions | 旧TUIには薄い | 実装済み | git metadata あり | 後続 |
| AI summary | 旧TUIに存在 | 現行でも文脈あり | summary infrastructure あり | 後続 |

## Why Not Revert gwt-cli

- 旧TUIコードは `tmux` multi-mode に深く結合している
- 現行 `gwt-core` は `terminal::*` と `session_store` / `session_watcher` に置換済み
- 単純リバートでは backend contract が崩れ、戻したあとに再度 large refactor が必要になる
- したがって、`旧TUIの UX` を抽出し、現行 backend の上で再構成するほうが筋が良い

## Cross-Spec Follow-Ups

| SPEC | Needed Sync |
|---|---|
| `SPEC-1654` | shell/session model を `tab-first` から `branch-first + permanent multi-mode` へ修正 |
| `SPEC-1770` | `Enter`、management toggle、grid/maximize 操作ポリシーを同期 |
| `SPEC-1777` | SPECs tab の parent navigation と launch entry を同期 |
| `SPEC-1782` | `1ブランチ = Nセッション` 前提へ Quick Start 導線を同期 |
| `SPEC-1542`, `SPEC-1656` | `Profiles = Env profiles` wording を必要なら明示 |

## Accepted Product Decisions

- 旧TUIの使いやすさは `branch list` と操作意味にある
- `tmux` は不要
- 新しい唯一の運用モデルは `permanent multi-mode`
- session workspace は `equal grid` を通常形、`maximize + tabs` を集中形にする
- 管理画面は `tabbed management workspace`
- 初回完成条件は `Branches / SPECs / Issues / Profiles`
- `AI summary` は後続
