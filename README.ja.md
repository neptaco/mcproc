# mcproc

AIエージェント上での快適なバックグラウンドプロセス管理を実現する Model Context Protocol (MCP) サーバーです。

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![Homebrew](https://img.shields.io/badge/Homebrew-tap%2Fneptaco-orange)](https://github.com/neptaco/homebrew-tap)

[English](README.md) | [日本語](README.ja.md)

## 概要

mcprocは、AIエージェント開発と従来のコマンドライン作業の間のギャップを埋めます。AIエージェントにより長時間実行される開発プロセス（開発サーバー、ビルドウォッチャーなど）を管理できるようにしながら、開発者はこれらの同じプロセスをモニタリング・制御するための完全なCLIアクセスを提供します。

## なぜmcprocが必要か？

単純なAIエージェントが起動するプロセスはステートレスで、長時間実行されるプロセスを効果的に管理できません。mcprocは以下の方法でこの問題を解決します：

- **統一された制御**: どのエージェント、どのターミナルで何が実行されているかの混乱がなくなります - すべてのプロセスが一元管理されます
- **コンテキストの保持**: ログがキャプチャされ保存されるため、AIエージェントは以前に発生した問題をログを確認しながらデバッグできます
- **開発者フレンドリー**: 完全なCLIアクセスにより、自分の開発環境から締め出されることはありません

## 主な機能

- 🔄 **統一されたプロセス管理**: MCPを介してAIエージェントからバックグラウンドプロセスを開始・管理し、ターミナルからモニタリング
- 👁️ **環境を超えた可視性**: AIエージェントが開始したプロセスはCLIや他のエージェントから完全にアクセス可能、その逆も同様
- 📝 **インテリジェントなログ管理**: 強力な正規表現パターンでプロセスログをキャプチャ、永続化、検索
- 📁 **プロジェクト対応**: プロジェクトコンテキストごとに自動的にプロセスをグループ化
- 📊 **リアルタイムモニタリング**: AIエージェントがプロセスを管理している間、CLIからリアルタイムでログを追跡
- 🛡️ **XDG準拠**: XDG Base Directory仕様に従った適切なファイル構成
- ⚡ **ログパターン待機**: プロセスを開始し、特定のログパターンを待機して準備完了を確認
- 🔍 **高度な検索**: ログ分析のための時間ベースのフィルタリング、コンテキスト行、正規表現サポート

## インストール

### Homebrew使用（macOSとLinux）

```bash
brew tap neptaco/tap
brew install mcproc
```

### ソースからビルド

```bash
# protobufコンパイラのインストール（必須）
# macOS:
brew install protobuf

# Ubuntu/Debian:
sudo apt-get install protobuf-compiler

# リポジトリのクローン
git clone https://github.com/neptaco/mcproc.git
cd mcproc

# ビルド
cargo build --release
```

### Claude Desktop統合

Claude Desktopの設定に追加:

```json
{
  "mcpServers": {
    "mcproc": {
      "command": "mcproc",
      "args": ["mcp", "serve"],
      "env": {}
    }
  }
}
```

## 使い方

mcprocはAIエージェントと開発者の両方に強力なインターフェースを提供します。

### AIエージェント向け（MCP）

AIエージェントは以下のMCPツールにアクセスできます:

- `start_process`: 開発サーバーまたはプロセスを開始
- `stop_process`: 実行中のプロセスを停止
- `restart_process`: プロセスを再起動
- `list_processes`: すべての管理されているプロセスを一覧表示
- `get_process_logs`: プロセスログを取得
- `search_process_logs`: 正規表現でログを検索
- `get_process_status`: 詳細なプロセス情報を取得

### 開発者向け（CLI）

AIエージェントがバックグラウンドでプロセスを管理している間、モニタリングと制御ができます：

おすすめコマンド: `mcproc logs -f`

#### CLIコマンド

| コマンド | 説明 | フラグ | 例 |
|---------|-------------|-------|---------|
| 🗒️ `ps` | すべての実行中プロセスを一覧表示 | `-s, --status <STATUS>` ステータスでフィルタ | `mcproc ps --status running` |
| 🚀 `start **<NAME>**` | 新しいプロセスを開始 | `-c, --cmd <CMD>` 実行するコマンド<br>`-d, --cwd <DIR>` 作業ディレクトリ<br>`-e, --env <KEY=VAL>` 環境変数<br>`-p, --project <NAME>` プロジェクト名<br>`--wait-for-log <PATTERN>` ログパターンを待機<br>`--wait-timeout <SECS>` 待機タイムアウト | `mcproc start web -c "npm run dev" -d ./app` |
| 🛑 `stop **<NAME>**` | 実行中のプロセスを停止 | `-p, --project <NAME>` プロジェクト名<br>`-f, --force` 強制終了 (SIGKILL) | `mcproc stop web -p myapp` |
| 🔄 `restart **<NAME>**` | プロセスを再起動 | `-p, --project <NAME>` プロジェクト名 | `mcproc restart web` |
| 📜 `logs **<NAME>**` | プロセスログを表示 | `-p, --project <NAME>` プロジェクト名<br>`-f, --follow` ログ出力を追跡<br>`-t, --tail <NUM>` 表示する行数 | `mcproc logs web -f -t 100` |
| 🔍 `grep **<NAME>** **<PATTERN>**` | 正規表現でログを検索 | `-p, --project <NAME>` プロジェクト名<br>`-C, --context <NUM>` コンテキスト行<br>`-B, --before <NUM>` マッチ前の行<br>`-A, --after <NUM>` マッチ後の行<br>`--since <TIME>` 指定時刻以降を検索<br>`--until <TIME>` 指定時刻以前を検索<br>`--last <DURATION>` 指定期間内を検索 | `mcproc grep web "error" -C 3` |
| 🎛️ `daemon start` | mcprocデーモンを開始 | なし | `mcproc daemon start` |
| 🎛️ `daemon stop` | mcprocデーモンを停止 | なし | `mcproc daemon stop` |
| 🎛️ `daemon status` | デーモンステータスを確認 | なし | `mcproc daemon status` |
| 🔌 `mcp serve` | MCPサーバーとして実行 | なし | `mcproc mcp serve` |
| ℹ️ `--version` | バージョン情報を表示 | なし | `mcproc --version` |
| ❓ `--help` | ヘルプメッセージを表示 | なし | `mcproc --help` |


#### 例

```bash
# デーモンを開始（まだ実行されていない場合）
mcproc daemon start

# すべてのプロセスを表示（AIエージェントが開始したものを含む）
mcproc ps

# リアルタイムでログを追跡
mcproc logs frontend -f

# プロジェクト単位でのマルチプロセスログ追跡
mcproc logs -f

# ログを検索
mcproc grep backend "error" -C 5

# プロセスを停止
mcproc stop frontend
```

### ワークフローの例

1. **AIエージェントが開発サーバーを開始:**
   ```
   エージェント: 「フロントエンドの開発サーバーを起動します」
   → MCPツールを使用: start_process(name: "frontend", cmd: "npm run dev", wait_for_log: "Server running")
   ```

2. **ターミナルからモニタリング:**
   ```bash
   mcproc logs -f
   # サーバーが実行されている間、リアルタイムでログを確認
   ```

3. **AIエージェントがエラーを検出してログを検索:**
   ```
   エージェント: 「エラーの原因を確認させてください」
   → MCPツールを使用: search_process_logs(name: "frontend", pattern: "ERROR|WARN", last: "5m")
   ```

4. **同じ情報を確認:**
   ```bash
   mcproc grep frontend "ERROR|WARN" -C 3 --last 5m
   ```

### 高度な例

```bash
# 環境変数を設定してプロセスを開始
mcproc start api --cmd "python app.py" --env PORT=8000 --env DEBUG=true

# プロセスが準備完了になるまで特定のログパターンを待機
mcproc start web --cmd "npm run dev" --wait-for-log "Server running on" --wait-timeout 60

# 時間フィルタでログを検索
mcproc grep api "database.*connection" --since "14:30" --until "15:00"

# 同じプロジェクト内の複数のプロセスからログを表示
mcproc ps
mcproc logs web --project myapp -t 100
```

## アーキテクチャ

mcprocは3つの主要コンポーネントで構成されています：

1. **mcprocd**: プロセスを管理し、ログの永続化を処理する軽量デーモン
2. **mcproc CLI**: デーモンと対話するための開発者向けコマンドラインインターフェース
3. **MCPサーバー**: Model Context Protocolを介してAIエージェントにプロセス管理機能を公開

### ファイルの場所（XDG準拠）

- **設定**: `$XDG_CONFIG_HOME/mcproc/config.toml` (デフォルト: `~/.config/mcproc/`)
- **ログ**: `$XDG_STATE_HOME/mcproc/log/` (デフォルト: `~/.local/state/mcproc/log/`)
- **ランタイム**: `$XDG_RUNTIME_DIR/mcproc/` (デフォルト: `/tmp/mcproc-$UID/`)

## 開発

### ソースからビルド

```bash
# リポジトリをクローン
git clone https://github.com/neptaco/mcproc.git
cd mcproc

# すべてのコンポーネントをビルド
cargo build --release

# テストを実行
cargo test

# 詳細なロギングで実行
RUST_LOG=mcproc=debug cargo run -- daemon start
```

### プロジェクト構造

```
mcproc/
├── mcproc/         # CLIとデーモンの実装
├── mcp-rs/         # 再利用可能なMCPサーバーライブラリ
├── proto/          # Protocol Buffer定義
└── docs/           # アーキテクチャと設計ドキュメント
```

## コントリビューション

コントリビューションを歓迎します！お気軽にPull Requestを送信してください。

## ライセンス

MIT License

Copyright (c) 2025 Atsuhito Machida (neptaco)

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.