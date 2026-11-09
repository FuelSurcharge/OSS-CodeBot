# 配布パッケージの生成と確認

現在は配布準備中です。本体のライセンス、Windows/macOSの実機起動、各OSの新規ユーザー環境での確認、署名・公証は完了していません。作成できることと配布可能であることを区別します。

## 形式と同梱物

| 対象target | 形式 | GUI / CLI |
|---|---|---|
| x86_64-pc-windows-msvc | ZIP | `bin/codebot-gui.exe` / `bin/codebot-cli.exe` |
| x86_64-apple-darwin | ZIP | `CodeBot.app` / `bin/codebot-cli` |
| aarch64-apple-darwin | ZIP | `CodeBot.app` / `bin/codebot-cli` |
| x86_64-unknown-linux-gnu | tar.gz | `CodeBot.sh`または`bin/codebot-gui` / `bin/codebot-cli` |

macOSのアプリには、`Contents/MacOS/codebot-cli`も含めます。説明・設定例・通知は`Contents/Resources/`に保存し、`.app`を移動しても保持します。ほかのOSではアーカイブのルートに説明と通知を置きます。対象OS・CPUの検証状況は[検証表](PLATFORM_VALIDATION.md)を参照してください。

GUI・CLI、Markdownの操作説明、公開用の`*.example.toml`だけ、Transformers用スクリプトとrequirements、Rust製Wasmサンプル、フォントのライセンス、依存ライブラリの通知と元のライセンス文書を同梱します。MPLの依存ライブラリのソースも同梱します。個人用`config/local.toml`、APIキー、会話、モデル、キャッシュをコピーする処理はありません。

Python・torch・Transformers・llama-server・モデルの重みは同梱しません。ルール診断には不要で、ローカルAIを使う場合だけ[追加導入](LOCAL_DIAGNOSTICS.md)が必要です。Transformersの実行ブリッジとフォント自体はRustバイナリにも埋め込まれています。

LinuxはX11またはWayland、OpenGLドライバー、glibc、libssl/libcrypto、libxkbcommonなどのシステムライブラリが必要です。Ubuntuの例は`libssl3 libxkbcommon0 libwayland-client0 libx11-6 libxcb1 libgl1 zlib1g libzstd1`です。実際の不足は`ldd bin/codebot-gui`で確認してください。静的バイナリやAppImageではありません。WindowsはMSVCのランタイム、macOSは対象CPUと対応OSを新規環境で確認する必要があります。

## ローカルで内部検証用パッケージを作る

Python 3.11以降・Rust stableが必要です。以下はLinux targetの例です。ほかのOSは対応するtargetへ変更し、そのOSのビルドツールを使います。

```sh
rustup target add x86_64-unknown-linux-gnu
cargo build --workspace --release --locked --target x86_64-unknown-linux-gnu
cargo install cargo-about --version 0.9.3 --locked --features cli
python scripts/collect_notices.py --target x86_64-unknown-linux-gnu --out target/notices.json
python scripts/package_release.py --target x86_64-unknown-linux-gnu --notices target/notices.json --development
```

`dist/CodeBot-<version>-development-<target>`のアーカイブと`.sha256`を生成します。同名の出力やチェックサムがある場合は上書きせず終了します。再生成時は別の`--out`を指定してください。生成はインストールや起動を伴いません。

`--binaries`でビルド済みバイナリの場所、`--profile debug`で内部検証用debug版を指定できます。ELF/PE/Mach-Oの種類とCPUを調べ、異なるtargetのバイナリを詰めた場合は失敗します。

依存通知の収集は`cargo-about --fail`を使います。ライセンスを解決できない場合は生成を停止します。収集したJSONはtarget・Cargo.lock・Cargo.toml・about.tomlのハッシュに結び付け、古い通知や別targetの通知を再利用しません。通知の収集にはcrateの公式ソースへアクセスすることがあります。追加依存やライセンス変更時には通知内容も確認してください。`about.toml`のacceptedは依存通知の生成用で、本体のライセンス選択ではありません。

`package-manifest.json`にはバージョン、target、profile、元コミット、未コミット変更の有無、ファイルごとのSHA-256を記録します。ビルド環境のパスを含む生のcargo-about JSONは配布物へコピーしません。バイナリをビルドしたコミットと、梱包するソースのコミットが一致することも確認してください。

## 一般配布に進む条件とCI

所有者が本体のライセンスを選んだ後、ルートの`LICENSE`と3つのcrateの`license`または`license-file`を設定します。ライセンス未決定のcrateは現在`publish = false`で、crates.ioへ公開しません。フォントや依存ライブラリにはそれぞれの条件が適用されます。

一般配布用の生成では`--development`を外します。本体のLICENSEとCargoメタデータ、release profile、コミット済みのソースが揃わない場合は生成を停止します。内部検証用には`LICENSE-STATUS.txt`を入れ、公開版と名前を分けます。

[packages.yml](../.github/workflows/packages.yml)は手動実行と`v<workspaceのversion>`タグを対象に4targetをビルドします。通常の手動実行は内部検証モードで、CI内で生成とテストだけを行い、配布アーカイブをアップロードしません。ライセンスが確定した通常モードではアーカイブとSHA-256をActionsの成果物へ保存します。タグ名とバージョンが一致しない場合も停止します。このワークフローはGitHub Releaseを作成・公開しません。

現在は署名なしです。Windowsのコード署名とmacOS Developer ID/公証は、所有者の証明書・公開方針が決まった後に追加します。署名済みとして表示せず、OSの保護機能を一括無効化する手順も採用しません。App Store、MSIX、DMG、deb/rpm、自動更新は最初のポータブル配布には含めません。

## 新規インストール・更新・削除の確認

1. チェックサムを照合する。Linuxは`sha256sum -c <archive>.sha256`、macOSは`shasum -a 256 -c <archive>.sha256`、Windowsは`Get-FileHash <archive> -Algorithm SHA256`の結果を同梱値と比較する。
2. 新規ユーザーの書き込み可能なフォルダへ展開し、GUIとCLIを起動する。macOSは対象CPUの`.app`を使い、説明・通知もアプリ内に残ることを確認する。
3. キー不要のルール診断、設定保存・再起動、選択したAPI/ローカルAIの導入と停止を確認する。IME・両フォント・小画面/高DPI・子プロセスも[検証表](PLATFORM_VALIDATION.md)へ記録する。
4. 更新はアプリを終了して、別フォルダへ新版を展開する。既存の設定・チャット・変更記録を保持したまま新版で確認する。旧版は確認が済むまで残す。ロールバック時に新版の設定を読める保証はないため、設定と変更記録を事前にバックアップする。
5. 削除はアプリ終了後に展開フォルダ/`.app`を削除する。OS標準の設定・会話・変更記録・モデルキャッシュは自動削除しない。必要に応じて内容とパスを確認して別に削除する。変更の原本を含む取り消し記録や共有モデルをまとめて消さない。

最低対応OS、署名、公証、新規ユーザーでの操作と配布CIの実行結果は未検証です。CB-017はこれらと本体ライセンスが揃うまで未完了のままにします。
