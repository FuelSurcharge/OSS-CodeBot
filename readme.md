# OSS CodeBot

Rust製のCLI・デスクトップGUI。共通コアで、APIキー不要の基本的なエラーログ診断を実行します。
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
sudo apt-get install build-essential pkg-config libxkbcommon-dev libwayland-dev libgl1-mesa-dev
```

GUIの「設定」にTOMLファイルのパスを入力すると、その設定を実行時に読み込みます。
未指定の場合はOS標準の設定ディレクトリ（directories crateのProjectDirs）内の
`config.toml`を読み込み、存在しなければ既定値を使います。
WindowsはユーザーのAppData、macOSはApplication Support、LinuxはXDG_CONFIG_HOME配下です。
明示した設定ファイルがない場合や設定が不正な場合はエラーになります。

## 現在の機能

- CLI引数／標準入力、GUIからの入力
- permission denied、not found、一般的なエラーの確認手順提示
- 共通TOML設定、文字数制限、プロバイダーtrait
- GUIのバックグラウンド処理、日本語フォント
- セッション一覧、診断履歴、結果コピー、Ctrl / ⌘ + Enterで診断
- 会話中心のGUI、折りたたみ可能なサイドバー、チャット別の下書き、画面内設定
- デフォルトは白基調。送信ボタンは青地に白い矢印、選択状態は淡い青で表示
- 狭いウィンドウではチャット切替メニューを使用。入力例はありません。
- 会話履歴はアプリ起動中のみ保持（終了時に消去）。診断は各入力単独で処理し、過去の会話は参照しません。
- 標準エラーへの処理完了／エラーログ（入力内容は記録しません）

AI回答、コード生成、GitHub連携、自動ファイル編集は未実装です。
ルールベースの案内は原因を確定するものではありません。

## 検証

```sh
cargo test --workspace --locked
cargo build --workspace --locked
```

3 OSのCI定義を同梱しています。GUIの起動・日本語入力・設定・エラー表示は各OSで別途操作検証が必要です。

設計は [docs/OSS_CodeBot_DESIGN.md](docs/OSS_CodeBot_DESIGN.md) を参照してください。
同梱のNoto Sans JPはSIL Open Font Licenseです。[assets/OFL.txt](assets/OFL.txt) を参照してください。
フォント取得元は google/fonts の ofl/notosansjp です。アプリ本体のライセンスは未決定です。

GUIの標準フォントは **LINE Seed JP Regular 400** です。「設定 → フォント」で **BIZ UDPゴシック** に変更でき、すぐに反映されます。選択はOS標準のアプリ設定ディレクトリにある `gui-preferences.toml` に保存され、次回起動にも復元されます。診断用TOMLとは別のファイルです。保存できない場合は設定画面にエラーを表示し、その起動中だけ反映します。

LINE Seed JPはGoogle Fonts公式の `ofl/lineseedjp`、BIZ UDPゴシックは `ofl/bizudpgothic` から取得しています。それぞれのSIL Open Font Licenseを [LINE Seed JP](assets/fonts/line-seed-jp/OFL.txt)・[BIZ UDPゴシック](assets/fonts/biz-udp-gothic/OFL.txt) に同梱しています。

現在のGUI方針は [DESIGN.md](DESIGN.md)、参照したSkillと適用範囲は [docs/ui-skills/README.md](docs/ui-skills/README.md) に記載しています。
