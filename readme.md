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
