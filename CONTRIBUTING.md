# 開発と不具合報告

Rust stable・Python 3.11以降を使います。Linuxのビルド依存は[README](readme.md)、ローカル推論の追加依存は[導入手順](docs/LOCAL_DIAGNOSTICS.md)を参照してください。アプリ本体のライセンスは未決定です。ライセンスを決めて公開配布するまで、配布物は内部検証用として扱います。

```sh
cargo fmt --all --check
cargo test --workspace --locked
cargo build --workspace --locked
python -m unittest discover -s scripts/transformers/tests -p 'test_*.py'
python -m unittest discover -s scripts/tests -p 'test_*.py'
```

GUIの変更では通常サイズ・700×540の実画面を確認します。IME変換は実際のOSの入力メソッドを使い、貼り付けだけで確認済みにしないでください。OS・ディスプレイ方式・CPU/GPU・実施内容と未実施内容を[操作検証表](docs/PLATFORM_VALIDATION.md)に記録します。

開発タスクは[AI-ISSUES.md](AI-ISSUES.md)と`docs/tasks/`で管理します。実装と検証した条件だけをチェックし、実API・モデル・各OSの未検証範囲を残してください。

不具合はGitHub Issuesに、再現手順・期待した動作・実際の動作・アプリのバージョン/コミット・OS/CPU・画面倍率・使用方式（API/GGUF/Transformers）を記載してください。起動できない場合は端末から実行して、秘密情報を除いたエラーと不足ライブラリを添付します。

APIキー・トークン・設定ファイルの秘密値・会話本文・非公開ソース・モデルの利用資格情報は報告に含めないでください。模擬データで再現するか、必要な部分を匿名化してください。スクリーンショットも公開してよい内容だけにします。

配布物の生成と検証は[配布手順](docs/RELEASING.md)を参照してください。
