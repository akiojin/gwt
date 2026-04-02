> **Canonical Boundary**: `SPEC-1770` は rebuilt TUI の input interaction policy の正本である。terminal emulation 自体は `SPEC-1541`、shell topology と parent UX は `SPEC-1654` / `SPEC-1776` が担当する。

# TUI マウス・キーボード操作

## Background

旧 `Agent Canvas` 前提の interaction spec は現在の rebuilt TUI と一致しない。新しい interaction policy では、`Branches` を起点にした navigation、`equal grid / maximize` session workspace、tabbed management workspace をキーボード中心で扱う。

## User Stories

### US-1: Branches から迷わず移動したい

ユーザーとして、`Branches` 一覧をキーボード中心で移動し、`Enter` で session か launch へ自然に入れるようにしたい。

### US-2: 複数 session を効率よく切り替えたい

ユーザーとして、grid 上の focus 移動、maximize toggle、maximize 時の tab switch を素早く行いたい。

### US-3: terminal 操作は現行の快適さを維持したい

ユーザーとして、ホイールスクロール、drag-copy、paste、live follow のような現行 PTY UX を失いたくない。

### US-4: 管理画面タブへ素早く出入りしたい

ユーザーとして、管理画面を開閉しつつ、`Branches / SPECs / Issues / Profiles` を同じ操作体系で行き来したい。

## Acceptance Scenarios

1. `Branches` でカーソル移動後、`Enter` が `no/one/many` で分岐する
2. `many sessions` のとき selector 上で既存 session 選択や追加起動へ移動できる
3. session workspace で focus session を maximize できる
4. maximize 時に tab switch で他 session を選べる
5. 管理画面を開閉しても session workspace の文脈を失わない
6. terminal 上で wheel scroll、drag-copy、paste が従来通り使える

## Functional Requirements

- FR-001: interaction はキーボード中心で全主要機能へ到達できなければならない
- FR-002: `Branches` 一覧は移動・選択・`Enter` 分岐を備えなければならない
- FR-003: `many sessions` selector はキーボードで完結しなければならない
- FR-004: session workspace は focus movement と maximize toggle を持たなければならない
- FR-005: maximize 時は tab switch shortcut を持たなければならない
- FR-006: 管理画面の open/close と tab navigation を同一体系で扱わなければならない
- FR-007: terminal 上の wheel scroll、drag-copy、paste、live follow behavior は維持しなければならない
- FR-008: `hidden pane` や canvas 固有 interaction を前提にしてはならない

## Non-Functional Requirements

- NFR-001: interaction policy はマウスなしでも操作可能でなければならない
- NFR-002: マウス操作はキーボード操作の補助であり、主要経路を置き換えてはならない
- NFR-003: terminal 向け mouse handling は `SPEC-1541` の runtime contract を壊してはならない

## Success Criteria

- SC-001: `Branches`、selector、session workspace、management tabs が一貫した shortcut policy で動く
- SC-002: terminal UX の快適さを落とさずに rebuilt shell model へ移行できる
