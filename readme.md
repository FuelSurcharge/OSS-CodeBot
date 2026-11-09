# OSS CodeBot

Rust製のCLI・デスクトップGUI。共通コアで、APIキー不要のログ診断、GGUF／公式safetensors形式のローカルAI、AI APIへの接続を実行します。
Windows・macOS・Linux向けの構成です。各OSでのGUI操作検証は未完了です。

## 実行

Rust stableをインストールして、リポジトリ直下で実行してください。

```sh
cargo run -p codebot-cli -- "Permission denied"
printf 'file not found' | cargo run -p codebot-cli
cargo run -p codebot-cli -- --config config/config.example.toml "エラー"
cargo run -p codebot-gui
```

CLIのみならGUIライブラリやディスプレイは不要です。GUIにはデスクトップ環境が必要です。
Linuxのビルド環境例（Debian/Ubuntu）:

```sh
sudo apt-get install build-essential pkg-config libxkbcommon-dev libwayland-dev libgl1-mesa-dev libssl-dev
```

GUIの「設定」にTOMLファイルのパスを入力すると、その設定を実行時に読み込みます。
未指定の場合はOS標準の設定ディレクトリ（directories crateのProjectDirs）内の
`config.toml`を読み込み、存在しなければ既定値を使います。
WindowsはユーザーのAppData、macOSはApplication Support、LinuxはXDG_CONFIG_HOME配下です。
明示した設定ファイルがない場合や設定が不正な場合はエラーになります。

ローカルAIは「設定 → 処理方式 → ローカルAI」から使います。「実行方式」でllama.cpp（GGUF）またはTransformers（公式形式）を選びます。前者には `llama-server`、後者にはPythonとTransformers環境が必要です。[ローカルモデル設定](docs/LOCAL_MODELS.md) を参照してください。

## 現在の機能

- CLI引数／標準入力、GUIからの入力
- permission denied、not found、一般的なエラーの確認手順提示
- 共通TOML設定、文字数制限、プロバイダーtrait
- GUIのバックグラウンド処理、日本語フォント
- セッション一覧、診断履歴、結果コピー、Ctrl / ⌘ + Enterで診断
- 会話中心のGUI、折りたたみ可能なサイドバー、チャット別の下書き、画面内設定
- 回答のMarkdown表示（見出し・リスト・表・引用・リンク）、言語別のコードハイライト、コードブロック単位のコピー
- デフォルトは白基調。送信ボタンは青地に白い矢印、選択状態は淡い青で表示
- 狭いウィンドウではチャット切替メニューを使用。入力例はありません。
- 会話履歴の送信はGUIで選択可能。送信先変更時には再確認し、ルール診断は常に今回の入力だけを処理します。
- CLIの`--interactive`で継続対話。`/clear`・`/trim`で送信履歴を減らせます。
- チャット・下書きの任意保存と復元、名前変更・検索・確認付き削除、Markdown/JSON書き出し。[使い方](docs/CONVERSATIONS.md)
- 作業フォルダのファイル・行範囲プレビューと明示的な添付、コード説明・エラー調査・レビュー、回答から参照行への移動。[使い方](docs/WORKSPACES.md)
- 構造化した変更案の差分表示、変更箇所を選んだ確認付き適用、競合検出、再起動後の取り消し。[使い方](docs/CHANGES.md)
- API・ローカルAIの回答を逐次表示、GUIの停止とCLIの`--stream`・Ctrl+C、途中回答の保持。[使い方](docs/STREAMING.md)
- ローカル実行環境・モデル形式・CPU/GPUの診断、実行ファイルとモデルの選択、専用仮想環境への導入・復旧手順。[使い方](docs/LOCAL_DIAGNOSTICS.md)
- GGUFのRange/ETag再開と分割パートの取得・検証、空き容量確認、使用中ロックと確認付きキャッシュ削除。[使い方](docs/MODEL_CACHE.md)
- APIのモデル一覧と確認付き接続診断、認証・権限・モデル・利用制限・応答形式のエラー分類。[使い方](docs/API_DIAGNOSTICS.md)
- OpenAI Responses API、4形式の共通ツール提案、変更/コマンドの個別確認、結果をAIへ送る前の確認、操作上限と循環検出。[使い方](docs/AI_TOOLS.md)
- WebAssembly拡張の導入前の権限表示、有効化/無効化/削除、選択した入力の処理、確認付きの変更・コマンド提案。Rust製サンプルと作者向けAPIを同梱。[使い方](docs/PLUGINS.md)
- 確認付きコマンド実行、標準出力・標準エラーの逐次表示、タイムアウト・停止・子プロセス終了、結果の下書き追加。[使い方](docs/COMMANDS.md)
- Gitの状態・差分・ブランチ、選択ステージ/解除、コミット文の提案と確認付きコミット・push。[使い方](docs/GIT.md)
- GitHubのIssue/PR一覧・本文・差分、チャットへの取り込み、Issueからのブランチ準備、確認付きIssue/PR作成・コメントと重複投稿防止。[使い方](docs/GITHUB.md)
- 標準エラーへの処理完了／エラーログ（入力内容は記録しません）

OpenAI・Gemini・Claude・DeepSeek・Qwen・GLM・Grok・MiMo・KimiとカスタムAPIに対応しています。設定画面で処理方式・モデルID・APIキー・接続先を指定してください。APIキーは保存せず、環境変数からも取得できます。詳しくは [API設定](docs/API.md) を参照してください。

GitHub連携はローカル模擬APIで検証済みです。本番GitHubへの認証・投稿と各社実APIの接続は別途検証が必要です。
ルールベースの案内は原因を確定するものではありません。

## 検証

```sh
cargo test --workspace --locked
cargo build --workspace --locked
```

3 OSのCI定義を同梱しています。GUIの起動・日本語入力・設定・エラー表示は各OSで別途操作検証が必要です。

検証対象と実施状況は[OS別の検証表](docs/PLATFORM_VALIDATION.md)、配布形式・生成・チェックサム・更新/削除は[配布手順](docs/RELEASING.md)、開発と不具合報告は[CONTRIBUTING](CONTRIBUTING.md)を参照してください。配布物の生成スクリプトと4targetのCI定義を追加していますが、本体のライセンスは未決定で、現在は内部検証用です。

設計は [docs/OSS_CodeBot_DESIGN.md](docs/OSS_CodeBot_DESIGN.md) を参照してください。
同梱のNoto Sans JPはSIL Open Font Licenseです。[assets/OFL.txt](assets/OFL.txt) を参照してください。
フォント取得元は google/fonts の ofl/notosansjp です。アプリ本体のライセンスは未決定です。

GUIの標準フォントは **LINE Seed JP Regular 400** です。「設定 → フォント」で **BIZ UDPゴシック** に変更でき、すぐに反映されます。選択はOS標準のアプリ設定ディレクトリにある `gui-preferences.toml` に保存され、次回起動にも復元されます。診断用TOMLとは別のファイルです。保存できない場合は設定画面にエラーを表示し、その起動中だけ反映します。

LINE Seed JPはGoogle Fonts公式の `ofl/lineseedjp`、BIZ UDPゴシックは `ofl/bizudpgothic` から取得しています。それぞれのSIL Open Font Licenseを [LINE Seed JP](assets/fonts/line-seed-jp/OFL.txt)・[BIZ UDPゴシック](assets/fonts/biz-udp-gothic/OFL.txt) に同梱しています。

現在のGUI方針は [DESIGN.md](DESIGN.md)、参照したSkillと適用範囲は [docs/ui-skills/README.md](docs/ui-skills/README.md) に記載しています。

今後の機能は[AI-ISSUES.md](AI-ISSUES.md)と[タスク別のMarkdown](docs/tasks/README.md)で管理します。優先度・進捗・完了条件・依存関係を記録し、実装後にファイルを更新してpushします。
