# クイックスタート: Docker Compose の Playwright noVNC を arm64 で起動可能にする

## 目的

arm64 環境で Playwright noVNC サービスを起動するための最小手順を示す。

## 手順

1. docker-compose.yml の playwright-novnc に platform 指定が追加されていることを確認する
2. arm64 環境で amd64 を実行する場合は、Docker のエミュレーションが有効であることを確認する
3. 必要に応じて環境変数 `PLAYWRIGHT_NOVNC_PLATFORM` を設定する
4. docker compose up -d を実行して起動確認する

## 期待される結果

- no matching manifest エラーが発生せず、playwright-novnc が起動できる
