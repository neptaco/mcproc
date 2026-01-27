---
name: release
description: 新しいバージョンをリリースする。「リリース」「release」「新バージョン」「バージョンアップ」と言われた場合に使用する。
disable-model-invocation: true
argument-hint: [version]
---

# リリースワークフロー

新しいバージョン **$ARGUMENTS** をリリースします。

## 前提条件

- main ブランチにいること
- 未コミットの変更がないこと
- CI が通っていること

## 手順

### 1. 事前確認

```bash
# 現在のブランチとステータス確認
git branch --show-current
git status --short
```

未コミットの変更がある場合は、先にコミットまたは stash してください。

### 2. バージョン決定

引数でバージョンが指定されていない場合、以下を確認：

```bash
# 最新のタグを確認
git describe --tags --abbrev=0 2>/dev/null || echo "タグなし"

# 前回リリースからの変更を確認
git log $(git describe --tags --abbrev=0 2>/dev/null)..HEAD --oneline
```

#### Semantic Versioning ガイド

| 変更内容 | バージョン | 例 |
|---------|----------|-----|
| 破壊的変更（BREAKING CHANGE） | メジャー | v1.0.0 → v2.0.0 |
| 新機能追加（feat:） | マイナー | v1.0.0 → v1.1.0 |
| バグ修正（fix:） | パッチ | v1.0.0 → v1.0.1 |

### 3. 品質チェック

```bash
cargo fmt -- --check && cargo clippy --all-targets -- -D warnings && cargo test
```

### 4. CHANGELOG 更新

CHANGELOG.md の `[Unreleased]` セクションを新しいバージョンに変更：

1. `## [Unreleased]` を `## [<VERSION>] - <TODAY>` に変更
2. ファイル末尾のリンク定義を更新

### 5. Cargo.toml バージョン更新

ワークスペースの Cargo.toml を更新：

```toml
[workspace.package]
version = "<VERSION>"
```

### 6. 変更をコミット

```bash
git add CHANGELOG.md Cargo.toml
git commit -m "chore: bump version to <VERSION>"
```

### 7. タグ作成とプッシュ

```bash
# タグを作成
git tag -a v<VERSION> -m "Release v<VERSION>"

# コミットとタグをプッシュ（GitHub Actions が自動でリリース）
git push origin main
git push origin v<VERSION>
```

### 8. リリース確認

GitHub Actions でリリースワークフローが実行される：

1. テスト実行
2. クロスプラットフォームビルド（Linux x86_64/arm64, macOS x86_64/arm64）
3. GitHub Release 作成（バイナリ添付）

```bash
# ワークフロー状態確認
gh run list --workflow=release.yml --limit 3
```

### 9. Homebrew 更新

リリースが完了したら、Homebrew Formula を更新：

```bash
# publish-homebrew ワークフローを手動実行
gh workflow run publish-homebrew.yml
```

または homebrew-tap リポジトリで手動更新。

### 10. リリース完了確認

```bash
# GitHub Release を確認
gh release view v<VERSION>

# または Web で確認
gh release view v<VERSION> --web
```

## 使用例

```bash
/release v0.1.4
/release v0.2.0
/release v1.0.0
```

## トラブルシューティング

### タグを削除したい場合

```bash
# ローカルタグ削除
git tag -d v<VERSION>

# リモートタグ削除（未リリースの場合のみ）
git push origin :refs/tags/v<VERSION>
```

### リリースをやり直したい場合

1. GitHub Release を削除
2. タグを削除（上記参照）
3. 修正をコミット
4. 再度タグ作成・プッシュ
