# 旧フォント比較用スナップショット

`line_seed_jp.rs` はLINE Seed JP Regular 400を評価した際の独立したGUIスナップショットです。
現在の通常版にもLINE Seed JPが標準採用され、「設定 → フォント」でBIZ UDPゴシックへ変更できます。
このexampleは当時の比較用として残してあり、通常版の設定機能には追従していません。

```sh
# 現在の通常版（標準LINE Seed JP、設定で切替可能）
cargo run -p codebot-gui --locked

# 旧比較用スナップショット（LINE Seed JP固定）
cargo run -p codebot-gui --example line_seed_jp --locked
```

フォントは通常版と共用の `assets/fonts/line-seed-jp/` に移動しました。
この共有フォントを削除すると通常版もビルドできなくなるため、exampleの削除時にも残してください。
配布元・ライセンスはルートのREADMEと共有フォルダのOFL.txtを参照してください。
