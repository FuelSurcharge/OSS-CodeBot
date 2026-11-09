# 入力の行数

渡した入力全体のLFによる行数を表示する、CodeBot拡張API v1のサンプルです。
配布単位は `manifest.json` と `plugin.wasm`。導入後に有効化し、入力を渡してください。
参照ファイルを添付する場合は `read-selected-files` の権限を使用します。
ファイル変更やコマンドの提案は返しません。

実装は [guest.rs](guest.rs)、ビルドはリポジトリルートから `python scripts/build_sample_plugin.py`。
再ビルド時はRustの `wasm32-unknown-unknown` ターゲットが必要です。
[導入・API仕様・権限・制約](../../docs/PLUGINS.md)を参照してください。
