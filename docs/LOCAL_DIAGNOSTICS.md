# ローカル実行環境の診断と初回セットアップ

GUIの「設定 → ローカルAI → 実行環境を診断」で、選択したPythonとllama-serverを実行して確認する。
モデルの取得・推論・パッケージの導入は行わない。
診断には選択した実行ファイルを使用するため、自分で導入した信頼できるファイルを指定する。

確認する項目:

- Pythonのバージョン、torch・transformers・accelerate・safetensors・huggingface_hubのバージョンとimport可否。同梱requirements.txtの範囲外も表示する。
- PyTorchによるCPU/CUDA/MPSの小さなテンソル割り当て。GPUはこの確認が通った場合に推論設定で選べる。利用不可の場合はCPUまたは別のPython環境を選ぶ。
- 取得可能なシステムメモリ合計・利用可能量、CUDAメモリ合計・空き、MPS推奨メモリ。取得できない値は表示しない。
- llama-serverの`--version`の実行可否と取得できたバージョン。GGUF v2/v3のヘッダー、またはconfig.jsonとsafetensorsの存在。

形式検証はモデル全体の互換性検証ではない。tokenizer・モデル種別・エンジン対応・必要メモリは実際の読み込み時にも確認する。
PyTorchで使えるGPUとllama.cppに組み込まれたGPUバックエンドは別のため、PyTorch診断でllama.cppのGPU対応を判定しない。
パス・形式を変更した後は再診断する。パッケージ更新・デバイス状態が変わった場合も再診断が必要。

Python・llama-server・GGUFファイル・モデルフォルダは選択画面から指定できる。手入力も継続して使用できる。
Python仮想環境の実行ファイルがシンボリックリンクの場合も選べる。

CLIは`--config 設定TOML --diagnose-local`で同じ診断結果をJSONで表示する。
API認証情報・環境変数値・モデル本文・実行ファイルの生のエラー出力は表示しない。
各プローブは15秒、標準出力・標準エラーはそれぞれ32 KiBを上限とし、キャンセル・失敗時は起動した診断プロセスを終了する。
Pythonは`-I`で実行し、ユーザーsite-packages・PYTHONPATHを含めない。目的の依存を導入した仮想環境を直接選ぶ。

## 導入・復旧

llama.cppは[公式リリース](https://github.com/ggml-org/llama.cpp/releases)からOS・CPU/GPUに合うパッケージを展開して実行ファイルを指定する。
Transformersは[ローカルモデルの手順](LOCAL_MODELS.md)に従い、新しい専用仮想環境へ依存を導入する。
依存の不整合がある場合も既存環境を一括更新せず、別の仮想環境を作ってPythonパスを切り替える。
CUDA用PyTorchは[公式の導入手順](https://pytorch.org/get-started/locally/)で対応構成を選ぶ。
設定画面にもOSに応じた仮想環境作成・導入コマンドと復旧手順を表示する。自動導入は行わない。

Linuxの実Python CPU環境で導入済みの依存と未導入のllama-server、合成safetensorsの形式、メモリを確認した。
テストでは未導入・import不整合・対応外バージョン・GPU割り当て失敗・出力量制限・キャンセル・機密的な例外値の除外を確認する。
Windows/macOSの独立環境とGPU実機は未検証。CB-005の3 OS検証条件は未完了のまま残している。
