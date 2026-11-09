# AI API設定

GUIの「設定 → 処理方式」から選びます。既定は外部送信しないローカル診断です。
API利用には各サービスのアカウント、利用可能なモデルID、APIキーが必要です。
モデル名はサービス側で変わるため自動選択せず、契約先が提供する正確なIDを入力します。

## 設定手順

1. 処理方式を選ぶ。
2. モデルIDを入力する。
3. ベースURLを確認する。地域や契約で異なる場合は変更する。
4. APIキーを入力するか、表示された環境変数に設定してアプリを起動する。
5. 「API設定を保存」でキー以外を保存し、チャットに戻って送信する。

入力したAPIキーはプロセスのメモリ内だけで保持し、ファイルへ保存しません。
処理方式や接続先の変更時には入力キーをクリアします。
APIキーが未入力なら、指定した名前の環境変数を参照します。
サービスごとのモデル・接続先などは、フォント設定と同じ `gui-preferences.toml` に保存します。
通常のTOML設定ファイルパスは入力制限などの読込用で、GUIで選んだAPI設定を優先します。

## 接続先プリセット

ベースURLには下表の値までを指定します。右のパスは自動で追加するため含めないでください。
プリセットは地域、アカウント、サービスの仕様更新に応じて調整してください。

| 処理方式 | ベースURL | 自動追加するパス | キー環境変数の既定名 |
|---|---|---|---|
| OpenAI | `https://api.openai.com/v1` | `/chat/completions` | `OPENAI_API_KEY` |
| Gemini | `https://generativelanguage.googleapis.com/v1beta` | `/models/{model}:generateContent` | `GEMINI_API_KEY` |
| Claude | `https://api.anthropic.com/v1` | `/messages` | `ANTHROPIC_API_KEY` |
| DeepSeek | `https://api.deepseek.com/v1` | `/chat/completions` | `DEEPSEEK_API_KEY` |
| Qwen | `https://dashscope-intl.aliyuncs.com/compatible-mode/v1` | `/chat/completions` | `DASHSCOPE_API_KEY` |
| GLM | `https://api.z.ai/api/paas/v4` | `/chat/completions` | `ZHIPU_API_KEY` |
| Grok | `https://api.x.ai/v1` | `/chat/completions` | `XAI_API_KEY` |
| MiMo | `https://api.xiaomimimo.com/v1` | `/chat/completions` | `MIMO_API_KEY` |
| Kimi | `https://api.moonshot.ai/v1` | `/chat/completions` | `MOONSHOT_API_KEY` |
| カスタム | 手動入力 | 選択した形式に従う | 手動入力 |

Qwenは国際向け、GLMはZ.ai向け、Kimiは国際向けのURLを初期値にしています。
中国本土向けの例は、Qwen `https://dashscope.aliyuncs.com/compatible-mode/v1`、
GLM `https://open.bigmodel.cn/api/paas/v4`、Kimi `https://api.moonshot.cn/v1` です。
アカウントとAPIキーの発行地域に一致する接続先を使ってください。

Geminiは `x-goog-api-key`、Claudeは `x-api-key` と `anthropic-version`、
MiMoは `api-key`、その他のOpenAI互換サービスはBearer認証を使用します。
OpenAIはChat Completions対応モデルを使用してください。Responses APIには未対応です。

## カスタムAPI

OpenAI互換・Gemini・Claude Messagesの3形式から選べます。
任意のREST APIのスキーマを自動変換する機能ではありません。
ローカルモデルサーバーには、例えば `http://127.0.0.1:1234/v1` を設定し、
必要に応じて「キーなし接続を許可」を有効にします。
HTTPはループバック接続のみ許可します。外部接続はHTTPSが必須です。
URLへの認証情報・クエリ・フラグメントの埋込みは受け付けません。
認証先が意図せず変わらないよう、HTTPリダイレクトは追従しません。

## CLI

[config/api.example.toml](../config/api.example.toml) のモデルIDなどを編集して使います。
APIキーは環境変数に設定してください。ファイルに値を書き込まないでください。

```sh
cargo run -p codebot-cli -- --config config/api.example.toml "このエラーを説明して"
```

`provider` の指定値は `rules`, `local`, `openai`, `gemini`, `claude`, `deepseek`, `qwen`,
`glm`, `grok`, `mimo`, `kimi`, `custom` です。
カスタムでは `[api]` の `protocol` に `openai`, `gemini`, `claude` のいずれかを指定します。

## 送信内容と確認範囲

送信ボタンを押すと、その入力テキストが選択先に送信されます。
GUIでは「会話履歴を含める」で過去の成功した会話も送信できます。初期値は無効です。CLIの`--interactive`では履歴を含めて送信します。
ローカルファイルは、作業フォルダと行範囲を選びプレビューして添付したものだけを送信します。リポジトリ全体を自動で送信しません。
詳しくは[継続対話・保存](CONVERSATIONS.md)と[ファイルの参照](WORKSPACES.md)を参照してください。
応答は完成後に表示します。利用料金、モデル可用性、データの扱いは契約先に従います。

HTTP認証・タイムアウト・JSON解析・空応答の失敗は画面または標準エラーに表示します。
キーやサーバーのエラー本文はログ出力しません。エラー表示はHTTPステータスなどに限定します。
TLS証明書検証は有効です。必要な環境ではOSの信頼済み証明書・プロキシ設定を使用してください。
Linuxのビルドには `pkg-config` と `libssl-dev` が必要です。

3形式とMiMoの認証ヘッダーをローカル模擬HTTPサーバーで検証し、GUI・CLIからも送受信を確認しました。
プリセットの各社アカウントを使った実API接続は、APIキーがないため未検証です。
特に各モデル固有の追加パラメーターやResponses API・ツール呼出しには未対応です。

ローカルGGUFモデルは [ローカルモデル設定](LOCAL_MODELS.md) を参照してください。
