1. current branch の既存 PR を列挙する
2. latest merged PR の merge commit が `HEAD` の祖先か確認する
3. 非祖先なら `origin/<head>..HEAD` を優先する
4. 0 commit なら `NO ACTION`、正の commit 数なら `CREATE PR` を返す
