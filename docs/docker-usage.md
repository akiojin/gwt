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
