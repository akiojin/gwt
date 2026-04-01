# SPECs タブ — ローカル SPEC 一覧・詳細・検索 UI

## Background

gwt-tui の管理画面に SPECs タブを追加し、ローカルの `specs/SPEC-*/` ディレクトリをスキャンして SPEC の一覧・詳細・検索を提供する。Issues タブとは完全に分離された独立タブとして実装する。gwt-spec-* スキルはエージェント向けの自動化ツールであり、本 UI は人間が直接 SPEC を閲覧・検索するためのインターフェース。

## User Stories

### US-1: SPEC 一覧を閲覧する

開発者として、管理画面の SPECs タブで全ローカル SPEC の一覧を確認し、状態（open/closed/in-progress）で素早くフィルタしたい。

### US-2: SPEC の詳細をプレビューする

開発者として、一覧から SPEC を選択し、spec.md の内容をプレビュー表示で確認したい。管理画面内で完結し、外部エディタを開く必要がないこと。

### US-3: SPEC を検索する

開発者として、SPEC のタイトルや内容をキーワード検索し、関連する SPEC を素早く見つけたい。

## Acceptance Scenarios

### AS-1: SPEC 一覧の表示

- gwt-tui の管理画面で SPECs タブを選択
- `specs/SPEC-*/metadata.json` がスキャンされ一覧表示される
- 各行に SPEC ID、タイトル、ステータス、優先度が表示される

### AS-2: ステータスフィルタ

- SPECs タブで一覧が表示されている状態
- ステータスフィルタ（All / Open / Closed / In-Progress）を切り替え
- フィルタに一致する SPEC のみが表示される

### AS-3: spec.md プレビュー

- SPEC 一覧から項目を選択
- 右ペイン（または詳細ビュー）に spec.md の内容がマークダウンプレビューで表示される

### AS-4: キーワード検索

- 検索フィールドにキーワードを入力
- タイトルおよび spec.md の内容に一致する SPEC がフィルタ表示される

### AS-5: Issues タブとの独立性

- SPECs タブと Issues タブは完全に分離されたタブ
- SPECs タブの操作が Issues タブの状態に影響しない

## Functional Requirements

| ID | 要件 |
|----|------|
| FR-001 | `specs/SPEC-*/metadata.json` をスキャンして SPEC 一覧を構築する |
| FR-002 | 一覧に SPEC ID、タイトル、ステータス、優先度、作成日を表示する |
| FR-003 | ステータス（open / closed / in-progress / all）でフィルタリングする |
| FR-004 | SPEC 選択時に spec.md の内容をプレビュー表示する |
| FR-005 | キーワード検索（タイトル + spec.md 内容）をサポートする |
| FR-006 | Issues タブとは完全に独立したタブとして実装する |
| FR-007 | 一覧のソート（ID順、ステータス順、優先度順）をサポートする |

## Non-Functional Requirements

| ID | 要件 |
|----|------|
| NFR-001 | SPEC が 100 件以上でもスキャン・表示が 500ms 以内に完了する |
| NFR-002 | spec.md のプレビュー表示は非同期読み込みで TUI をブロックしない |
| NFR-003 | ファイルシステム監視で SPEC の追加・変更を自動検出する（オプション） |

## Success Criteria

| ID | 基準 |
|----|------|
| SC-001 | 管理画面に SPECs タブが表示され、SPEC 一覧が正しく表示される |
| SC-002 | ステータスフィルタが機能し、フィルタ結果が正しい |
| SC-003 | SPEC 選択時に spec.md がプレビュー表示される |
| SC-004 | キーワード検索で関連 SPEC が見つかる |
| SC-005 | Issues タブと SPECs タブが独立して動作する |
