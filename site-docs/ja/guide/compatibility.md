---
title: 対応範囲・検証表・合成セッションサンプル
description: 7つの記録形式と再開・内蔵チャットの対応、実装照合とCLI実行の違い、コマンドで検証する合成JSONL・SQLite例を確認できます。
---

# 対応する記録形式は何ですか

下表の形式に対応します。**実装の照合と合成コマンドのテストは、CLI のエンドツーエンド認証ではありません。** opencode 行には今回の v2 保存形式への対応を追加しています。他の行は 2026-10-05 に確認した 0.6.0 の実装です。すべてのリリースへの保証ではありません。

## 能力と検証表 {#support-matrix}

7 つとも履歴閲覧・検索・エクスポート・ターミナル再開に対応します。内蔵チャットは Claude Code と Codex のみです。再開には対応する CLI のインストールと設定が必要です。[コマンド比較](/ja/agents/#resume-commands)を参照してください。

| エージェント | 対象形式 | 内蔵チャット | 検証の根拠 | 今回実行検証した CLI 版 |
| --- | --- | --- | --- | --- |
| [Claude Code](/ja/agents/claude-code) | JSONL `message.content` の文字列／ブロック | 対応 | 合成抽出＋[Rust 解析投影](#offline-parser-tests) | 未検証 |
| [Codex](/ja/agents/codex) | `session_meta`、`event_msg`、`response_item` | 対応 | 合成 `cwd`／本文＋[Rust 解析投影](#offline-parser-tests) | 未検証 |
| [Grok Build](/ja/agents/grok-build) | `updates.jsonl` の ACP 更新 | 非対応 | 合成オブジェクト／配列チャンク | 未検証 |
| [Kimi Code](/ja/agents/kimi-code) | 主 wire イベント、実装には旧形式の代替処理もある | 非対応 | 主イベントの合成例のみ | 未検証 |
| [Pi](/ja/agents/pi) | ヘッダーとメッセージ／ツリー | 非対応 | 保存された全分岐の合成テキスト | 未検証 |
| [Antigravity CLI](/ja/agents/antigravity-cli) | `transcript*.jsonl` の step | 非対応 | 合成の複数行 XML | 未検証 |
| [opencode](/ja/agents/opencode) | 旧 SQLite と v2 `session_v2` / `session_message` / 任意の `session_part` | 非対応 | 合成 DB の読み取り専用 SQL ＋ Rust 解析回帰 | 未検証 |

macOS、Windows、Linux のインストーラを提供しますが、シェル例は macOS/Linux 構文で、各 OS の GUI 認証ではありません。対応する Antigravity には使用量フィールドがありません。旧形式、別保存先、画像、ツール、分岐、プロバイダの挙動は各リファレンスと[確認手順](/ja/guide/troubleshooting)を参照してください。

## 安全に再現できるサンプル {#synthetic-samples}

最小の**合成テキスト抽出データ**であり、実際の会話や再開可能な完全なセッションではありません。画像／ツールの任意フィールドは省略しているため、実際の履歴としてインポート・再開しないでください。パスと入力は架空です。JSONL／SQL と<a href="/examples/expected.json" download>期待出力</a>は `npm run docs:test` と同じ入力です。

ダウンロード後、対応するリファレンスの抽出コマンドのファイル名を置き換えます。出力順は次のとおりです。

| エージェント | ダウンロード | 期待テキスト |
| --- | --- | --- |
| Claude Code | <a href="/examples/claude-code.jsonl" download>claude-code.jsonl</a> | `Question`、`Answer`、`Legacy string` |
| Codex | <a href="/examples/codex.jsonl" download>codex.jsonl</a> | `Question`、`Answer`。`cwd` は `/synthetic/project` |
| Grok Build | <a href="/examples/grok-build.jsonl" download>grok-build.jsonl</a> | `Question`、`Answer` |
| Kimi Code | <a href="/examples/kimi-code.jsonl" download>kimi-code.jsonl</a> | `Question`、`Answer`、`Legacy string` |
| Pi | <a href="/examples/pi.jsonl" download>pi.jsonl</a> | `Question`、`Answer`、`Other branch`。現在の分岐のみではない |
| Antigravity CLI | <a href="/examples/antigravity-cli.jsonl" download>antigravity-cli.jsonl</a> | `Question`、`on two lines`。1 つの複数行入力 |
| opencode（旧形式） | <a href="/examples/opencode.sql" download>opencode.sql</a> | 照会から `user` と `Question` |
| opencode（v2） | <a href="/examples/opencode-v2.sql" download>opencode-v2.sql</a> | 行は `user`、`assistant` 順。解析結果は `Question`、分割パートの `Answer` |

`claude-code.jsonl` を取得した例：

```bash
jq -r 'select(.type=="user" or .type=="assistant")
  | .message.content
  | if type=="string" then . else .[]? | select(.type=="text") | .text end' \
  claude-code.jsonl
```

```text
Question
Answer
Legacy string
```

opencode SQL は縮小したスキーマを作ります。**新しい一時 DB** だけに読み込み、実際の opencode DB には使わないでください。

```bash
temp_dir="$(mktemp -d)"
sqlite3 "$temp_dir/synthetic.db" < opencode.sql
sqlite3 "file:$temp_dir/synthetic.db?mode=ro" \
  "SELECT json_extract(data, '$.text') FROM part WHERE json_extract(data, '$.type') = 'text';"
```

出力は `Question` です。テストでは親一覧が子を除外し、コスト合計に子を含め、読み取り後に DB が不変であることも確認します。実際の請求や opencode 移行の検証ではありません。

v2 サンプルは上流のセッション関連テーブルの一部で、完全なネイティブ DB ではありません。新しい一時 DB のみに読み込んでください。[v2 リファレンス](/ja/agents/opencode)に照会とリネーム・ゴミ箱移動・復元の制限を記載しています。

## オフライン解析テストが示す範囲 {#offline-parser-tests}

3 つの検証範囲を区別します。

1. **`jq`／SQL 抽出**：`npm run docs:test` が上記の小さなサンプルに文書のコマンドを実行します。内容／SEO 回帰と画像の完全性も確認します。コマンド出力はアプリの解析結果ではありません。
2. **実際の Rust ファイル解析器**：2026-10-06、macOS で 0.6.0 の Claude Code／Codex の解析と後処理を隔離した 2 テストが通りました。公開の期待投影と、role、本文、Claude thinking、ツール名／引数／ID／結果、メッセージ順序を照合し、一時入力が不変であることも確認します。Codex には空のタイトル索引を渡し、個人の索引を読みません。home 探索、CLI／ツール実行、プロバイダ通信はありません。
3. **実際の CLI／GUI 全工程の受け入れ**：今回は未実施です。画像、閲覧／検索／書き出し／再開、インストール済み CLI の版、プロバイダ挙動はこの例で認証しません。表に実行確認済みの CLI 版は追加していません。

| オフライン解析の入力 | 期待メッセージ |
| --- | --- |
| <a href="/examples/claude-code-adapter.jsonl" download>claude-code-adapter.jsonl</a> | ユーザー質問 → assistant の thinking／本文／ツール呼び出し → user の結果 → 旧式ユーザー文字列 |
| <a href="/examples/codex-adapter.jsonl" download>codex-adapter.jsonl</a> | ユーザー質問 → assistant のツール呼び出し → user の結果 → assistant の回答。重複 response 本文と無関係なメタデータを除外 |

<a href="/examples/adapter-expected.json" download>adapter-expected.json</a>に正確な照合フィールドがあります。これらもインポート／再開可能な完全な CLI 会話ではありません。`echo synthetic` などのツール引数はデータで、実行しません。

Rust／Tauri の依存関係をキャッシュ済みの環境で、リポジトリのルートから実行します。

```bash
npm run docs:test
npm run docs:test:adapters
```

追加した OpenCode v2 の隔離解析テストは、上記の公開 SQL 例を使い、NULL のタイトル、Windows 形式のパス、メッセージ順、分割パート、コストと token を確認します。OpenCode や GUI は実行しません。

実際のコマンドは `cargo test --manifest-path src-tauri/Cargo.toml --lib --locked --offline docs_public_fixture`。この 3 テストを実行し、crate のキャッシュがなければ取得せず失敗します。既存の Rust CI に含まれ、Node 文書 CI は Rust に依存しません。ローカルの再現可能な根拠であり、公開やインデックスの証明ではありません。

## 形式の変更を報告するには {#report-change}

アプリ／CLI の版、OS、形式、別保存先、失敗した操作を記載します。最小の合成／匿名化例を使い、認証情報や個人の DB 全体は公開しないでください。実際にその版と操作を実行した場合のみ実行検証済みとします。

根拠：[アダプター版 22fefc6](https://github.com/jerrywu001/cc-sessions-viewer/tree/22fefc6/src-tauri/src/agents)。[プライバシー](/ja/guide/privacy)、[インストール](/ja/guide/install)、[保守について](/ja/guide/about)も参照できます。
