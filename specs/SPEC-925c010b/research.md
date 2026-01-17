# 調査メモ: Docker Compose の Playwright noVNC を arm64 で起動可能にする

## 現状

- docker-compose.yml の playwright-novnc は `ghcr.io/xtr-dev/mcp-playwright-novnc:latest` を参照している
- arm64 環境では docker compose up -d 実行時に「no matching manifest for linux/arm64/v8」が発生する

## 原因整理

- docker compose はホストのアーキテクチャを優先して pull する
- 対象イメージが arm64 マニフェストを提供していない場合、pull 時点で失敗する

## 方針

- compose 定義に platform を追加し、既定で linux/amd64 を指定する
- environment 変数で platform を上書き可能にして、将来的な arm64 対応や個別事情に対応する
- arm64 向けにエミュレーション要件をドキュメントへ明記する
