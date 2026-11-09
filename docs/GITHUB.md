# GitHub Issue・Pull Request

GUI上部の「GitHub」から対象を `owner/repository` で指定します。Gitのリモートとは独立した設定です。接続先とリポジトリを確認してから利用してください。

## 読み込みとチャット

「接続を確認」でリポジトリと既定ブランチを取得します。「参照」でIssue/PRを切り替え、一覧を取得するか番号を指定して本文を読み込みます。一覧はopenの対象を1ページ50件、明示したページだけ取得します。PRでは本文と差分を表示します。

本文と差分はチャットの既存下書きへ追加できます。12,000文字を超える資料は切り詰めを明示します。AIへは自動送信しません。Issueからのブランチは `issue-番号` を準備し、Git画面で作業フォルダ・現在のブランチ・参照元を確認して作成します。現在のHEADから分岐し、既存の変更を保持します。GitHubからの自動fetchやbaseブランチへの切り替えは行いません。

## 認証と投稿

「認証と接続先」でトークンを入力できます。入力した値は起動中のメモリだけで使用します。未入力の場合は指定した環境変数（既定 `GH_TOKEN`）を参照します。公開対象の読み込みには認証なしでも利用できます。非公開対象へのアクセスとIssue/PRの書き込みには、そのリポジトリへの権限が必要です。fine-grained tokenでは対象リポジトリとIssues/Pull requestsの必要な権限をGitHub側で設定してください。組織の承認・SSOが別途必要な場合もあります。

「接続設定を保存」はAPI URL・対象リポジトリ・環境変数名・待機時間だけを保存します。トークン、投稿本文は通常の設定や送信記録に保存しません。API URLを変更すると入力済みトークンと環境変数名をクリアします。

「投稿」でIssue作成・PR作成・コメントを選びます。PRは既定で下書きです。送信元headと対象baseはGitHub上に存在するブランチを指定します。この画面はローカルブランチのpushを代行しません。送信先と本文のプレビュー後、確認にチェックして投稿します。変更した内容には再確認が必要です。

APIはHTTPS（ローカル検証用loopbackのみHTTPも可）で、既定は `https://api.github.com` です。EnterpriseではRESTルート（例 `https://host/api/v3`）を指定します。リダイレクトや応答中のURLを認証付きでたどりません。Webリンクは選択したホスト直下のリポジトリパスから構築します。独自のWebパス接頭辞がある構成は未対応です。

## 送信記録と結果が不明な場合

OS標準のアプリ設定ディレクトリの `github-posts/` に、投稿前に記録を作ります。記録はAPI URL・対象リポジトリ・操作内容のSHA-256・結果の番号です。キーや本文は記録しません。「送信記録」で再読み込みできます。

同じ接続先・リポジトリ・操作・本文の再投稿は、アプリ再起動後やCLIの別プロセスからも拒否します。GitHub側の冪等性キーを使う方式ではありません。本文やタイトルを変えると別の操作になるため、修正して再投稿する前にも一覧を確認してください。

- `confirmed`：成功応答を確認しました。
- `pending-or-uncertain`：送信前の記録、通信切断、停止、タイムアウト、応答形式の不一致などで結果が不明です。GitHubの一覧で確認してください。自動再投稿はしません。
- 認証・権限・404・利用制限・入力不正の応答は分類して表示します。既知の拒否応答では記録を取り除き、修正後に新しくプレビューできます。

記録を消すと重複防止が失われます。結果不明のまま削除しないでください。保存できない場合は投稿を開始しません。成功後の記録更新に失敗した場合も、投稿結果を表示し、再投稿せずGitHubでの確認を案内します。

本文は256 KiB、PR差分は1 MiB、通常のJSON応答は2 MiB、投稿タイトルは256文字・本文は65,536文字を上限とします。上限超過は省略して投稿せず、エラーにします。送信記録の一覧は1,000件までです。トークンを含み得るHTTPエラー本文は表示しません。

## CLI

```sh
codebot-cli --github-repository owner/repo --github-check
codebot-cli --github-repository owner/repo --github-list
codebot-cli --github-repository owner/repo --github-kind pull --github-read 43
codebot-cli --github-repository owner/repo --workspace /absolute/work --github-branch 42 --github-preview
codebot-cli --github-repository owner/repo --github-write /absolute/post.json --github-preview
codebot-cli --github-repository owner/repo --github-write /absolute/post.json
codebot-cli --github-repository owner/repo --github-history
```

投稿JSONの形式:

```json
{"operation":"create-issue","title":"修正の件名","body":"本文"}
```

```json
{"operation":"create-pull","title":"修正の件名","body":"本文","head":"feature/fix","base":"main","draft":true}
```

```json
{"operation":"comment","number":42,"body":"本文"}
```

`--github-preview`では書き込みません。通常実行では表示した内容を確認して `post`（ブランチ作成は `git`）と入力します。`--yes`はその確認を明示的に省略します。Ctrl+Cで通信を中止できますが、投稿開始後は結果不明になることがあります。`--github-api-url`・`--github-token-env`・絶対パスの `--github-ledger-dir` を指定できます。トークンの値をコマンド引数に渡すオプションはありません。

## 検証範囲

2026-11-09、Rustのローカル模擬HTTPテストで読み込み・3種類の投稿・権限/レート制限・切断・不正応答・事前キャンセル・重複拒否・保存内容を確認しました。CLIの別プロセス間でも同一投稿の再送を拒否しました。
Linuxの実GUIでIssue/PR本文と差分、下書き保持、確認付きローカルブランチ作成、模擬サーバーへの投稿を操作しています。
本番GitHubの認証・投稿、Enterprise実機、Windows/macOSの実画面は未検証です。このクラウドの通信許可に `api.github.com` が含まれないため、本番APIへの到達は確認できていません。
