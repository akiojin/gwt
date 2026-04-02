# Plan: SPEC-1776 — 旧TUI UX を基準にした ratatui TUI 再設計

## Summary

`SPEC-1776` は「旧TUIの UX を現行 backend 上で再構成する親 SPEC」として扱う。単純な `gwt-cli` 復元は採らず、`gwt-core` の `native PTY`、hooks、SPEC/Issue integration を再利用しながら、`branch list` 中心の UX、常設マルチモード、管理画面タブ、`Profiles = Env profiles` を再設計する。

## Cross-Spec Inventory

| Concern | Canonical SPEC | Plan での扱い |
|---|---|---|
| Parent UX / migration sequencing | `SPEC-1776` | 比較マトリクス、初回完成条件、child sync 順序を持つ |
| Terminal emulation | `SPEC-1541` | `native PTY + vt100` を維持し、scrollback / selection / resize 契約を流用 |
| Interaction policy | `SPEC-1770` | `Enter`、管理画面トグル、grid/maximize 操作へ再配線 |
| Workspace shell | `SPEC-1654` | `tab-first` 記述を `branch-first + permanent multi-mode` に同期する follow-up を切る |
| Local git / worktree | `SPEC-1644` | branch-first UX でも local git/worktree owner はそのまま参照する |
| Agent catalog / launch contract | `SPEC-1646` | Wizard / launch selector は child contract を参照する |
| Session persistence | `SPEC-1648` | `1ブランチ = Nセッション` でも persistence owner は変えない |
| SPECs tab | `SPEC-1777` | tab form 維持。一覧・詳細・launch entry の親遷移だけ合わせる |
| Quick Start | `SPEC-1782` | Branches 起点の再開導線を維持し、multi-session selector に接続する |
| Hooks merge | `SPEC-1786` | launch flow の child 契約として接続 |
| SPEC workflow / storage | `SPEC-1579` | local artifact 正本を維持 |
| Workspace initialization | `SPEC-1787` | Branches / SPEC-first 導線との整合を確認 |
| Issue detail | `SPEC-1354` | Issues detail rendering は child 正本を維持する |
| Issue search / discovery | `SPEC-1643` | Issues list/search の parent dependency として監査する |
| Issue linkage / cache | `SPEC-1714` | Branch row / Issues detail の source of truth として再利用 |
| Profiles persistence | `SPEC-1542`, `SPEC-1656` | env profile 保存形を再利用 |
| Assistant semantics | `SPEC-1636` | Shell/Assistant interrupt semantics を上書きしない |
| Custom agents | `SPEC-1779` | 初回は後続だが owner として inventory に含める |

## Design Decisions

- 旧TUIの価値は `branch list` を中心に置いた情報設計と操作意味にある
- `tmux` の実装は捨てるが、複数セッション運用の価値は `permanent multi-mode` に置換する
- Session surface は通常時 `equal grid`、集中時 `maximize + tab switch`
- 管理画面は `tabbed management workspace` を採用し、初回は `Branches / SPECs / Issues / Profiles` に絞る
- `Profiles` は環境変数プロファイル専用とし、一般設定は後続 `Settings` フェーズへ回す
- branch-first core の安定後は、既存実装を活かして `Settings`、`Versions`、`Logs` を順次再露出する
- `Profiles` は env 専任のまま維持し、`Settings` には environment category を戻さない
- `AI summary` は引き続き後続フェーズ

## Architecture Direction

### gwt-core reuse

- `terminal::*`
- `agent::launch`
- `session_store` / `session_watcher`
- `config::skill_registration`
- `git::issue` / `git::issue_spec` / `git::local_spec`
- `ProfilesConfig` と関連 persistence

### gwt-tui redesign targets

- `branch list` を中心にした entry surface
- `1ブランチ = Nセッション` を扱う selector と session index
- `equal grid / maximize` を持つ session workspace
- `tabbed management workspace`
- `Profiles` 独立タブ

## Phased Implementation

### Phase 0: Parent Spec Reset

- `SPEC-1776` の spec / plan / tasks / research / data-model / quickstart を新前提へ更新
- `旧TUI / 現行 gwt-tui / 現行 gwt-core / 新目標` の比較マトリクスを作る
- child SPEC へ波及する更新箇所を inventory 化する
- workflow / persistence / integration 系を含め、関連 SPEC を `sync required / reference only / deferred` に分類する

### Phase 1: Branch-First Shell Model

- `Branches` を唯一の primary entry に戻す
- ブランチ行の session count 表示を追加する
- `Enter` を `no session / one session / many sessions` の 3 分岐へ作り直す
- `hidden pane` なしの session index を作る

### Phase 2: Permanent Multi-Mode Session Workspace

- `equal grid` を通常レイアウトにする
- `4件以上` を前提に layout rules を作る
- focus session の maximize toggle を作る
- maximize 時の tab switch を作る
- 管理画面開閉と layout restore を接続する

### Phase 3: Management Workspace Core

- `Branches / SPECs / Issues / Profiles` の 4 タブへ整理する
- `SPECs / Issues` は一覧・詳細・launch entry を維持する
- `Profiles` は env profile 管理に絞る
- `Settings` は non-env categories (`General / Worktree / Agent / Custom / AI`) を扱う

### Phase 4: Launch Flow Integration

- multi-session aware な branch enter flow を作る
- `既存へ入る / 追加起動 / フルWizard` selector を作る
- `Quick Start` を新しい selector と両立させる
- hooks confirm と skill registration を launch path へ再接続する

### Phase 5: Child SPEC Synchronization

- `SPEC-1654` を新 shell model に合わせる
- `SPEC-1770` を新 shortcut / layout policy に合わせる
- `SPEC-1777` を management tabs と launch entry に同期する
- `SPEC-1782` を `1ブランチ = Nセッション` 前提へ同期する
- `SPEC-1579` / `SPEC-1787` の workflow entry contract が branch-first UX と矛盾しないか監査する
- `SPEC-1714` / `SPEC-1354` / `SPEC-1643` の Issue list/detail/linkage contract を監査する
- `SPEC-1786` の hooks confirm が新しい branch enter selector と矛盾しないか監査する
- 必要に応じて `SPEC-1542` / `SPEC-1656` の profile wording を見直す
- `SPEC-1636` / `SPEC-1779` / `SPEC-1648` / `SPEC-1646` / `SPEC-1644` は reference-only 監査対象として整合確認する

### Deferred Phase

- `AI summary`
- custom agent UI refresh

## Verification Baseline

- artifact update: `markdownlint` と diff review
- implementation start gate:
  - comparison matrix が揃っている
  - child sync list が揃っている
  - `Branches / Session / Wizard / SPECs / Issues / Profiles` の受け入れ条件が tasks に落ちている
- code verification when implementation starts:
  - `cargo test -p gwt-core -p gwt-tui`
  - `cargo clippy --all-targets --all-features -- -D warnings`
  - Branches → Session → Wizard → SPECs/Issues → Profiles の manual walkthrough
