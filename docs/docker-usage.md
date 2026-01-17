# Docker環境での利用

## エラー対応

### `exec /entrypoint.sh: no such file or directory` エラーが発生した場合

このエラーは通常、改行コードの問題で発生します。以下の手順で解決してください：

1. **Dockerイメージの再ビルド**

   ```bash
   # 既存のコンテナとイメージを削除
   docker-compose down
   docker system prune -f

   # イメージを再ビルド
   docker-compose build --no-cache
   ```

2. **コンテナの起動**

   ```bash
   docker-compose up -d
   ```

3. **コンテナに接続**
   ```bash
   docker-compose exec gwt bash
   ```

## トラブルシューティング

### 改行コードの問題

Windowsで開発している場合、シェルスクリプトの改行コードがCRLFになることがあります。
`.gitattributes`ファイルが設定されているため、Gitで管理されるファイルは自動的にLFに変換されます。

手動で修正する場合：

```bash
# macOS/Linuxの場合
perl -pi -e 's/\r\n/\n/g' .docker/entrypoint.sh

# ファイルタイプを確認
file .docker/entrypoint.sh
# 正常な出力: "Bourne-Again shell script text executable"
# (CRLF line terminatorsが含まれていないこと)
```

### arm64 環境で `no matching manifest for linux/arm64/v8` が発生する場合

playwright-novnc のイメージが arm64 マニフェストを提供していない場合、arm64 環境では pull 時点で失敗します。以下の手順で amd64 を指定して起動してください。

1. 環境変数を設定

   ```bash
   export PLAYWRIGHT_NOVNC_PLATFORM=linux/amd64
   ```

   もしくは `.env` に以下を追加します。

   ```text
   PLAYWRIGHT_NOVNC_PLATFORM=linux/amd64
   ```

2. コンテナ起動

   ```bash
   docker compose up -d
   ```

> 注意: arm64 上で amd64 を実行するため、Docker Desktop のエミュレーションや qemu-user-static などが有効である必要があります。無効な場合は `exec format error` が発生します。

### Dockerイメージのクリーンアップ

完全にクリーンな状態から始める場合：

```bash
# すべてのコンテナを停止・削除
docker-compose down -v

# Dockerシステムの完全クリーンアップ
docker system prune -a --volumes -f

# 再ビルド
docker-compose build --no-cache
docker-compose up -d
```
