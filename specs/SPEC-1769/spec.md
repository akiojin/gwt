> **🔄 TUI MIGRATION (SPEC-1776)**: This SPEC has been updated for the gwt-tui migration. Image display in TUI uses sixel or kitty graphics protocol instead of GUI-based image rendering.

# TUI 画像表示（sixel/kitty）

## Background

gwt-tui のターミナル上で画像を表示するために、**sixel グラフィックスプロトコル** または **kitty グラフィックスプロトコル** を検討する。エージェントが生成したスクリーンショット、アーキテクチャ図、UI モックアップなどをターミナル内で直接確認できるようにする。

ターミナルエミュレーターの対応状況:

| プロトコル | 対応ターミナル |
|-----------|---------------|
| Sixel | xterm, mlterm, WezTerm, foot, iTerm2 |
| Kitty graphics | kitty, WezTerm |
| iTerm2 inline images | iTerm2, WezTerm |

## User Stories

### US-1: エージェント出力の画像をターミナルに表示する

ユーザーとして、エージェントが生成・取得した画像（スクリーンショット、図表など）をターミナル内で直接表示し、コンテキスト切り替えなしに確認したい。

### US-2: 画像ファイルをプレビュー表示する

ユーザーとして、ローカルの画像ファイルを TUI 内でプレビュー表示したい。

### US-3: ターミナルの対応プロトコルを自動検出する

ユーザーとして、自分のターミナルが対応しているグラフィックスプロトコルを自動検出し、最適な方式で画像を表示してほしい。

## Acceptance Scenarios

### AS-1: sixel 対応ターミナルでの画像表示

- sixel 対応ターミナル（例: WezTerm）で gwt-tui を起動
- エージェントがスクリーンショットを生成
- ターミナル内に画像がインライン表示される

### AS-2: kitty graphics 対応ターミナルでの画像表示

- kitty で gwt-tui を起動
- 画像ファイルのプレビューを要求
- kitty graphics protocol で画像が表示される

### AS-3: 非対応ターミナルでのフォールバック

- グラフィックスプロトコル非対応のターミナルで gwt-tui を起動
- 画像表示を要求
- ASCII/ブロック文字によるフォールバック表示、またはファイルパスのみ表示

## Functional Requirements

- FR-1: ターミナルのグラフィックスプロトコル対応を自動検出する（sixel / kitty / iTerm2 / なし）
- FR-2: 対応フォーマット: PNG, JPG (JPEG), SVG (ラスタライズ), WebP, GIF, BMP
- FR-3: 検出されたプロトコルに応じて最適な方式で画像をインライン表示する
- FR-4: 非対応ターミナルではフォールバック表示（ファイルパス表示 + 外部ビューア起動オプション）
- FR-5: 画像の永続化: ローカルファイルはパス参照、クリップボード等は `~/.gwt/images/` にコピー保存

## Non-Functional Requirements

- NFR-1: 大きな画像ファイル（50 MB 以上）でも非同期読み込みで TUI をブロックしない
- NFR-2: 画像表示が他の TUI 操作のパフォーマンスに影響しない
- NFR-3: SVG はラスタライズして表示する

## Success Criteria

- SC-1: sixel 対応ターミナルで画像がインライン表示される
- SC-2: kitty で kitty graphics protocol による画像表示が動作する
- SC-3: 非対応ターミナルでフォールバック表示が機能する
- SC-4: グラフィックスプロトコルの自動検出が正しく動作する
