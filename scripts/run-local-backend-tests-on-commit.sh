#!/usr/bin/env bash
set -euo pipefail

# Pre-commit: fast, non-IO-bound tests only.
# Coordination/board/persistence tests share state with running gwt and may
# fail when the app is active.  Full suite runs in pre-push / CI.

# git が hook 子プロセスへ export する GIT_DIR / GIT_INDEX_FILE などは、
# cargo test 配下の subprocess (tempdir に対する git コマンドなど) に
# 継承されると、tempdir 操作が本リポジトリ自身に向いてしまい、tests が
# 並列実行時に flaky になる。pre-commit のテストはリポジトリ自身の git
# 状態に依存しないので、ここで一律に unset しておく。
unset GIT_DIR GIT_INDEX_FILE GIT_WORK_TREE GIT_PREFIX \
  GIT_OBJECT_DIRECTORY GIT_ALTERNATE_OBJECT_DIRECTORIES \
  GIT_NAMESPACE GIT_COMMON_DIR

cargo test -p gwt-core -- \
  --skip coordination \
  --skip repo_hash::tests::detect_repo_hash
