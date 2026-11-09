# ローカルAI：GGUFと公式形式

既存の「ローカル診断」は単純なルールベース処理です。
「ローカルAI」は、GGUF向けのllama.cpp方式と、公式safetensors向けのTransformers方式から選べます。
まずllama.cpp方式について説明します。推論には別途 [llama.cppのllama-server](https://github.com/ggml-org/llama.cpp/tree/master/tools/server) が必要です。
導入状況はGUIの「実行環境を診断」またはCLIの`--diagnose-local`で確認できます。[診断・復旧手順](LOCAL_DIAGNOSTICS.md)。
各OS向けに配布・ビルドされた実行ファイルを用意し、GUIでパスを設定してください。
エンジンの自動インストールは行いません。モデルのアーキテクチャが使用するllama.cppに対応している必要があります。

## ローカルファイルから

1. 設定で「処理方式 → ローカルAI」「実行方式 → llama.cpp（GGUF）」を選ぶ。
2. 既存GGUFファイルのフルパスを「ローカルモデルのパス」に入力する。
3. `llama-server` の実行ファイルパスを指定する。PATHに登録済みなら既定名でよい。
4. 「読み込む」を押し、読み込み済み表示を待つ。
5. チャットに戻って送信する。

ローカルパスのファイルは上書きしません。GGUF v2/v3のヘッダーを確認してからエンジンを起動します。
ファイルを最終的に解釈するのはllama.cppです。形式不一致・メモリ不足・起動失敗・タイムアウトは画面に表示します。
初期値はCPU 4スレッド、GPUレイヤー0、コンテキスト4096です。
高度な設定で変更できますが、必要なRAM/VRAMはモデル・量子化・コンテキストに依存します。
「停止」、他の処理方式への切り替え、アプリ終了時には、アプリが起動した推論プロセスを終了します。
既存の別アプリの推論プロセスを終了することはありません。

## Hugging Face URLから

リポジトリのトップページではなく、GGUFファイルのURLを指定します。
次のOWNER/REPOなどは説明用のプレースホルダーで、実在モデルのプリセットではありません。

```text
https://huggingface.co/OWNER/REPO/resolve/main/model.gguf
https://huggingface.co/OWNER/REPO/blob/main/model.gguf
```

「URLからダウンロード」でバックグラウンド取得し、SHA-256が一致したファイルだけをOS標準キャッシュ内の`models`へ配置します。
成功後はそのパスを自動設定するので「読み込む」で起動してください。
取得時はHugging Faceの配布メタデータでコミットとサイズを固定します。SHA-256はLFSメタデータから取得し、取得できない単一GGUFでは利用者が配布元のSHA-256を入力します。
ユーザー指定値と配布元の値が異なる場合、検証が不一致の場合は使用しません。
既存キャッシュも再検証します。キャッシュからオフラインで使用する場合はローカルパスを指定してください。

取得サイズの初期上限は64 GiB。必要に応じて変更できます。
取得前に空き容量を確認します。モデルのRAM/VRAM要件・ライセンスも事前に確認してください。
HFの利用同意が必要なモデルはHugging Face側で同意してから、HFトークン欄または環境変数`HF_TOKEN`を使用します。
トークンはGUI設定ファイルには保存せず、huggingface.co以外の転送先にも送信しません。
URLにトークンを埋め込む形式は受け付けません。

取得量・進捗・キャンセルを表示します。通信応答待ちの間はキャンセル完了まで時間がかかる場合があります。
通信中断・キャンセル時は部分ファイルを残し、再実行時にETagとRangeを検証して再開します。ETagや配布内容が変わった場合は古い取得内容を流用しません。
分割GGUFは先頭の00001パートのURLから全パートを同じコミットで取得・検証できます。GUI/CLIのキャッシュ一覧で容量・空き容量・使用中の状態を確認し、選択した対象を確認後に削除できます。[再開・分割・キャッシュの詳細](MODEL_CACHE.md)。

## モデル名の選択について

以下を表示名として選べます。

- Gemma 4
- DeepSeek v4 Flash 0731
- GLM 5.3
- GLM 5.3 Flash
- Qwen 3.7 Plus
- カスタムモデル

これらの指定名について、正確な公開リポジトリ、重みの入手可否、GGUF配布、llama.cpp対応は確認できていません。
架空のダウンロード先を埋め込まず、パス・URLは利用者が指定する仕様です。
モデル名の選択は対応や動作を保証しません。モデルを変更したときは古いモデルのパスとURLをクリアします。
API専用・未公開のモデルはローカルでは読み込めません。
llama.cpp方式はsafetensorsの直接読み込みには未対応です。公式形式を使う場合は後述のTransformers方式を選択してください。
分割GGUFは5桁の連番形式に対応します。自分で配置する場合も全パートを同じフォルダへ置き、先頭ファイルを指定してください。

## CLI

[config/local.example.toml](../config/local.example.toml) を自分のパスへ変更して使います。

```sh
cargo run -p codebot-cli -- --config config/local.example.toml "このエラーを説明して"
```

CLIは1回の要求ごとにサーバーを起動・終了します。GUIは読み込み後もサーバーを保持します。
CLIで`model_path`を空にし、`hf_url`に有効なGGUF URLを指定した場合は、取得・検証後に読み込みます。
GUI/CLIとも、推論要求は127.0.0.1にだけ送信し、一時的な認証キーでアプリが起動したサーバーへ接続します。
GUIは「会話履歴を含める」を有効にしたときに、そのチャットの過去の成功した会話も推論へ渡します。CLIは`--interactive`で継続対話できます。

## 検証範囲と現在の環境制限

モデル保存のSHA-256検証・サイズ上限・キャンセル、ローカルプロセスの起動・認証・応答・終了は、テスト用GGUFヘッダーと模擬サーバーで検証しています。
llama.cpp方式の実モデル重みによる推論は未検証です。このクラウドにはllama-serverが未導入で、HFへの接続もHTTP 403で拒否されています。
モデル名プリセットの配布URLは、利用者からの正確なURL提示または公式配布元を確認してから設定します。
Hugging Face向け通信先の追加はクラウド環境のドラフトへ保存済みですが、現在のインスタンスに適用されたことは確認していません。
環境設定で変更を確認・保存し、環境を公開してから実モデルのダウンロード確認が必要です。

## GGUFのない公式形式（Transformers）

「ローカルAI」の実行方式で **Transformers（公式形式）** を選択すると、GGUFへの変換なしで使えます。
対応対象は、インストール済みTransformersの`AutoModelForCausalLM`が標準対応しているsafetensorsモデルです。
config.json、tokenizer、safetensors重み（分割も可）が揃ったフォルダを指定してください。
単一のsafetensorsファイルだけでは読み込めません。
マルチモーダル専用クラス、Pickle形式のpytorch_model.bin、配布リポジトリ独自コードを必須とするモデルは対象外です。
モデル名の表示用プリセットは、引き続き特定のモデルの動作を保証しません。

### Python環境

初回のみPython 3.10以降の独立環境を作ります。アプリが勝手にPythonパッケージをインストールすることはありません。
macOS/Linuxの例:

```sh
python3 -m venv .venv-transformers
.venv-transformers/bin/python -m pip install -r scripts/transformers/requirements.txt
```

Windowsでは `python -m venv .venv-transformers` を実行し、以降は `.venv-transformers\Scripts\python.exe` を使ってください。
CPUだけを使うLinux/Windows環境では、先にPyTorch公式のCPU用wheelを導入するとGPU用の大きな依存を避けられます。

```sh
.venv-transformers/bin/python -m pip install --index-url https://download.pytorch.org/whl/cpu 'torch>=2.6,<3'
.venv-transformers/bin/python -m pip install -r scripts/transformers/requirements.txt
```

GUIの「Python実行ファイル」には、この環境のpythonのフルパスを設定します。
CUDAを使う場合は、GPUドライバーに合うPyTorchを公式の手順でインストールしてください。
`auto`は利用可能ならCUDA、次にMPS、なければCPUを選びます。CUDAのauto配置はAccelerateを使用します。
CPU/CUDA/MPSの個別指定も可能です。CUDA・MPSの実機検証は未実施です。

### 公式リポジトリからの取得

Transformers方式では、Hugging FaceリポジトリのURLまたは `owner/repo` のIDを指定します。
リビジョンの初期値はmainで、取得時は正確なコミットに固定して全ファイルを取得します。
設定・tokenizer・safetensors重み・ライセンス等のデータのみを取得し、`.py`や`.bin`は取得対象にしません。
LFSファイルはSHA-256、小さいGitファイルは配布メタデータのGit blob hashと照合します。
ファイルサイズ合計を確認して取得上限を適用します。サイズや検証情報がない場合は検証を省略せず失敗にします。

取得にはhuggingface_hubを使用し、部分キャッシュは次回の再開用に残します。
HFトークンはメモリから子プロセスの標準入力へ渡し、設定ファイル・コマンド引数・ログに保存しません。
取得したモデルからの独自コード実行（trust_remote_code）は許可しません。
読み込み時はローカルファイルのみを使用し、推論時に重みを勝手に外部取得しません。
コンパニオンPythonプログラムはアプリに同梱し、ハッシュ照合したものをキャッシュへ配置して使用します。

### CLIと検証

[config/transformers.example.toml](../config/transformers.example.toml) を自分のフォルダ・Pythonパスへ変更します。

```sh
cargo run -p codebot-cli -- --config config/transformers.example.toml "質問"
```

Linux CPUで、小型のランダム重みsafetensorsモデルを作成して実際のロード・生成・GUI/CLI応答を確認しています。
これは学習済みモデルの品質・指定された大規模モデルの互換性の検証ではありません。
検証時の環境: PyTorch 2.15.0+cpu、Transformers 5.22.1、Accelerate 1.15.0、safetensors 0.8.0、huggingface_hub 1.33.0。
Hugging Faceへの実接続は、以前からの通信制限により未検証です。
標準ライブラリだけで動く検証は `python -m unittest discover -s scripts/transformers/tests -p 'test_*.py'` で実行できます。
