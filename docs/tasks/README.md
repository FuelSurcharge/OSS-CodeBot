# 機能開発タスク

2026-11-05作成。実装基準は [8647f96](https://github.com/FuelSurcharge/OSS-CodeBot/commit/8647f96)。
管理先はGitHub Issues。各MarkdownはIssueへ登録する本文で、登録前の状態ではIssue番号を持たない。

既存のRust共通コア・CLI/GUI・日本語フォント設定・9社とカスタムAPI接続・GGUF/Transformersのローカル推論は、再作成の対象に含めていない。
各機能の基盤実装と、各社実API・大規模モデル・各OS実機での検証完了を区別してタスク化した。

## 開発順

P0は次の開発対象、P1は開発支援・運用の拡張、P2は基本機能の後に進める候補。
独自OS・ドライバーは既存ロードマップの将来構想として分離し、アプリの次期リリースには含めない。
期限・担当者は未設定。

| ID | 優先度 | 作る機能 | 主な前提 |
|---|---|---|---|
| [CB-001](CB-001.md) | P0 | 会話履歴を含む継続対話・CLI対話モード | なし |
| [CB-002](CB-002.md) | P0 | ストリーミング・生成停止 | 共通イベント型 |
| [CB-003](CB-003.md) | 完了 | Markdown・コード表示・コピー（2026-11-09） | なし |
| [CB-005](CB-005.md) | P0 | ローカル推論の診断・初回セットアップ | 各OSの推論環境 |
| [CB-006](CB-006.md) | P0 | 配布元・互換性を確認したモデルカタログ | 公式配布情報・検証資源 |
| [CB-008](CB-008.md) | P0 | 作業フォルダ・ファイル読み込み | なし |
| [CB-016](CB-016.md) | P0 | 3 OSのGUI・日本語IME・アクセシビリティ検証 | 各OSのデスクトップ |
| [CB-004](CB-004.md) | P1 | チャット保存・再開・整理・書き出し | CB-001 |
| [CB-007](CB-007.md) | P1 | モデル取得再開・分割GGUF・キャッシュ管理 | 既存取得処理 |
| [CB-009](CB-009.md) | P1 | ファイルを使う説明・ログ解析・レビュー | CB-008 |
| [CB-010](CB-010.md) | P1 | 変更案の差分確認・選択適用・取り消し | CB-008、CB-009 |
| [CB-011](CB-011.md) | P1 | 確認付きコマンド実行・テスト結果 | CB-008 |
| [CB-012](CB-012.md) | P1 | Gitの状態・差分・ブランチ・コミット | CB-008、CB-010 |
| [CB-013](CB-013.md) | P1 | アプリからGitHub Issue・PRを扱う | CB-012 |
| [CB-014](CB-014.md) | P1 | API接続診断・モデル一覧 | 各社の接続仕様 |
| [CB-017](CB-017.md) | P1 | 3 OS配布パッケージ・リリース手順 | CB-005、CB-016、ライセンス決定 |
| [CB-015](CB-015.md) | P2 | Responses API・ツール呼び出し | CB-001、CB-002、CB-010、CB-011 |
| [CB-018](CB-018.md) | P2 | 権限を明示するプラグイン拡張 | CB-010、CB-011、CB-015 |
| [FUT-OS-001](FUT-OS-001.md) | 将来構想 | 独自OSの仕様・最小起動のタスク分割 | 対象・目的の決定 |
| [FUT-DRV-001](FUT-DRV-001.md) | 将来構想 | 独自ドライバーの対象・APIのタスク分割 | FUT-OS-001・OSの基盤 |

着手順は「継続対話・逐次表示・コード表示」「ローカル推論を使い始める導線」「ファイル読み込み→レビュー→差分適用→Git/GitHub」の順を推奨する。
3 OSの操作検証はこれらと並行して進める。

## GitHubの画面からIssueを作る

現在はGit操作でこのタスク一覧を公開し、Issue本体の登録を保留している。下表の「作成画面」を開くと、タイトルと仕様へのリンクが入力された状態になる。既存Issueを検索して重複がないことを確認し、GitHubの画面で作成ボタンを押す。リンクを開くだけではIssueは作成されない。

| タスク | Issue作成 |
|---|---|
| [CB-001](CB-001.md) | [作成画面](https://github.com/FuelSurcharge/OSS-CodeBot/issues/new?title=%5BCB-001%5D+%E4%BC%9A%E8%A9%B1%E5%B1%A5%E6%AD%B4%E3%82%92%E5%90%AB%E3%82%80%E7%B6%99%E7%B6%9A%E5%AF%BE%E8%A9%B1%E3%82%92GUI%E3%83%BBCLI%E3%81%AB%E8%BF%BD%E5%8A%A0%E3%81%99%E3%82%8B&body=%3C%21--+codebot-task%3ACB-001+--%3E%0A%0A%E4%BB%95%E6%A7%98%E3%83%BB%E5%84%AA%E5%85%88%E5%BA%A6%E3%83%BB%E5%AE%8C%E4%BA%86%E6%9D%A1%E4%BB%B6%E3%83%BB%E4%BE%9D%E5%AD%98%E9%96%A2%E4%BF%82%3A%0Ahttps%3A%2F%2Fgithub.com%2FFuelSurcharge%2FOSS-CodeBot%2Fblob%2Fmain%2Fdocs%2Ftasks%2FCB-001.md) |
| [CB-002](CB-002.md) | [作成画面](https://github.com/FuelSurcharge/OSS-CodeBot/issues/new?title=%5BCB-002%5D+%E5%BF%9C%E7%AD%94%E3%81%AE%E3%82%B9%E3%83%88%E3%83%AA%E3%83%BC%E3%83%9F%E3%83%B3%E3%82%B0%E8%A1%A8%E7%A4%BA%E3%81%A8%E7%94%9F%E6%88%90%E5%81%9C%E6%AD%A2%E3%82%92%E8%BF%BD%E5%8A%A0%E3%81%99%E3%82%8B&body=%3C%21--+codebot-task%3ACB-002+--%3E%0A%0A%E4%BB%95%E6%A7%98%E3%83%BB%E5%84%AA%E5%85%88%E5%BA%A6%E3%83%BB%E5%AE%8C%E4%BA%86%E6%9D%A1%E4%BB%B6%E3%83%BB%E4%BE%9D%E5%AD%98%E9%96%A2%E4%BF%82%3A%0Ahttps%3A%2F%2Fgithub.com%2FFuelSurcharge%2FOSS-CodeBot%2Fblob%2Fmain%2Fdocs%2Ftasks%2FCB-002.md) |
| [CB-003](CB-003.md) | [作成画面](https://github.com/FuelSurcharge/OSS-CodeBot/issues/new?title=%5BCB-003%5D+Markdown%E3%83%BB%E3%82%B3%E3%83%BC%E3%83%89%E3%83%96%E3%83%AD%E3%83%83%E3%82%AF%E3%82%92%E8%AA%AD%E3%81%BF%E3%82%84%E3%81%99%E3%81%8F%E8%A1%A8%E7%A4%BA%E3%81%99%E3%82%8B&body=%3C%21--+codebot-task%3ACB-003+--%3E%0A%0A%E4%BB%95%E6%A7%98%E3%83%BB%E5%84%AA%E5%85%88%E5%BA%A6%E3%83%BB%E5%AE%8C%E4%BA%86%E6%9D%A1%E4%BB%B6%E3%83%BB%E4%BE%9D%E5%AD%98%E9%96%A2%E4%BF%82%3A%0Ahttps%3A%2F%2Fgithub.com%2FFuelSurcharge%2FOSS-CodeBot%2Fblob%2Fmain%2Fdocs%2Ftasks%2FCB-003.md) |
| [CB-004](CB-004.md) | [作成画面](https://github.com/FuelSurcharge/OSS-CodeBot/issues/new?title=%5BCB-004%5D+%E3%83%81%E3%83%A3%E3%83%83%E3%83%88%E3%81%AE%E4%BF%9D%E5%AD%98%E3%83%BB%E5%86%8D%E9%96%8B%E3%83%BB%E5%90%8D%E5%89%8D%E5%A4%89%E6%9B%B4%E3%83%BB%E5%89%8A%E9%99%A4%E3%83%BB%E6%9B%B8%E3%81%8D%E5%87%BA%E3%81%97%E3%82%92%E8%BF%BD%E5%8A%A0%E3%81%99%E3%82%8B&body=%3C%21--+codebot-task%3ACB-004+--%3E%0A%0A%E4%BB%95%E6%A7%98%E3%83%BB%E5%84%AA%E5%85%88%E5%BA%A6%E3%83%BB%E5%AE%8C%E4%BA%86%E6%9D%A1%E4%BB%B6%E3%83%BB%E4%BE%9D%E5%AD%98%E9%96%A2%E4%BF%82%3A%0Ahttps%3A%2F%2Fgithub.com%2FFuelSurcharge%2FOSS-CodeBot%2Fblob%2Fmain%2Fdocs%2Ftasks%2FCB-004.md) |
| [CB-005](CB-005.md) | [作成画面](https://github.com/FuelSurcharge/OSS-CodeBot/issues/new?title=%5BCB-005%5D+%E3%83%AD%E3%83%BC%E3%82%AB%E3%83%AB%E6%8E%A8%E8%AB%96%E7%92%B0%E5%A2%83%E3%81%AE%E8%A8%BA%E6%96%AD%E3%81%A8%E5%88%9D%E5%9B%9E%E3%82%BB%E3%83%83%E3%83%88%E3%82%A2%E3%83%83%E3%83%97%E3%82%92%E8%BF%BD%E5%8A%A0%E3%81%99%E3%82%8B&body=%3C%21--+codebot-task%3ACB-005+--%3E%0A%0A%E4%BB%95%E6%A7%98%E3%83%BB%E5%84%AA%E5%85%88%E5%BA%A6%E3%83%BB%E5%AE%8C%E4%BA%86%E6%9D%A1%E4%BB%B6%E3%83%BB%E4%BE%9D%E5%AD%98%E9%96%A2%E4%BF%82%3A%0Ahttps%3A%2F%2Fgithub.com%2FFuelSurcharge%2FOSS-CodeBot%2Fblob%2Fmain%2Fdocs%2Ftasks%2FCB-005.md) |
| [CB-006](CB-006.md) | [作成画面](https://github.com/FuelSurcharge/OSS-CodeBot/issues/new?title=%5BCB-006%5D+%E9%85%8D%E5%B8%83%E5%85%83%E3%81%A8%E4%BA%92%E6%8F%9B%E6%80%A7%E3%82%92%E7%A2%BA%E8%AA%8D%E3%81%A7%E3%81%8D%E3%82%8B%E3%83%AD%E3%83%BC%E3%82%AB%E3%83%AB%E3%83%A2%E3%83%87%E3%83%AB%E3%82%AB%E3%82%BF%E3%83%AD%E3%82%B0%E3%82%92%E4%BD%9C%E3%82%8B&body=%3C%21--+codebot-task%3ACB-006+--%3E%0A%0A%E4%BB%95%E6%A7%98%E3%83%BB%E5%84%AA%E5%85%88%E5%BA%A6%E3%83%BB%E5%AE%8C%E4%BA%86%E6%9D%A1%E4%BB%B6%E3%83%BB%E4%BE%9D%E5%AD%98%E9%96%A2%E4%BF%82%3A%0Ahttps%3A%2F%2Fgithub.com%2FFuelSurcharge%2FOSS-CodeBot%2Fblob%2Fmain%2Fdocs%2Ftasks%2FCB-006.md) |
| [CB-007](CB-007.md) | [作成画面](https://github.com/FuelSurcharge/OSS-CodeBot/issues/new?title=%5BCB-007%5D+%E3%83%A2%E3%83%87%E3%83%AB%E5%8F%96%E5%BE%97%E3%81%AE%E5%86%8D%E9%96%8B%E3%83%BB%E5%88%86%E5%89%B2GGUF%E3%83%BB%E3%82%AD%E3%83%A3%E3%83%83%E3%82%B7%E3%83%A5%E7%AE%A1%E7%90%86%E3%82%92%E8%BF%BD%E5%8A%A0%E3%81%99%E3%82%8B&body=%3C%21--+codebot-task%3ACB-007+--%3E%0A%0A%E4%BB%95%E6%A7%98%E3%83%BB%E5%84%AA%E5%85%88%E5%BA%A6%E3%83%BB%E5%AE%8C%E4%BA%86%E6%9D%A1%E4%BB%B6%E3%83%BB%E4%BE%9D%E5%AD%98%E9%96%A2%E4%BF%82%3A%0Ahttps%3A%2F%2Fgithub.com%2FFuelSurcharge%2FOSS-CodeBot%2Fblob%2Fmain%2Fdocs%2Ftasks%2FCB-007.md) |
| [CB-008](CB-008.md) | [作成画面](https://github.com/FuelSurcharge/OSS-CodeBot/issues/new?title=%5BCB-008%5D+%E4%BD%9C%E6%A5%AD%E3%83%AA%E3%83%9D%E3%82%B8%E3%83%88%E3%83%AA%E3%81%AE%E9%81%B8%E6%8A%9E%E3%81%A8%E3%83%95%E3%82%A1%E3%82%A4%E3%83%AB%E8%AA%AD%E3%81%BF%E8%BE%BC%E3%81%BF%E3%82%92%E8%BF%BD%E5%8A%A0%E3%81%99%E3%82%8B&body=%3C%21--+codebot-task%3ACB-008+--%3E%0A%0A%E4%BB%95%E6%A7%98%E3%83%BB%E5%84%AA%E5%85%88%E5%BA%A6%E3%83%BB%E5%AE%8C%E4%BA%86%E6%9D%A1%E4%BB%B6%E3%83%BB%E4%BE%9D%E5%AD%98%E9%96%A2%E4%BF%82%3A%0Ahttps%3A%2F%2Fgithub.com%2FFuelSurcharge%2FOSS-CodeBot%2Fblob%2Fmain%2Fdocs%2Ftasks%2FCB-008.md) |
| [CB-009](CB-009.md) | [作成画面](https://github.com/FuelSurcharge/OSS-CodeBot/issues/new?title=%5BCB-009%5D+%E3%83%95%E3%82%A1%E3%82%A4%E3%83%AB%E3%82%92%E4%BD%BF%E3%81%86%E3%82%B3%E3%83%BC%E3%83%89%E8%AA%AC%E6%98%8E%E3%83%BB%E3%83%AD%E3%82%B0%E8%A7%A3%E6%9E%90%E3%83%BB%E3%83%AC%E3%83%93%E3%83%A5%E3%83%BC%E6%93%8D%E4%BD%9C%E3%82%92%E8%BF%BD%E5%8A%A0%E3%81%99%E3%82%8B&body=%3C%21--+codebot-task%3ACB-009+--%3E%0A%0A%E4%BB%95%E6%A7%98%E3%83%BB%E5%84%AA%E5%85%88%E5%BA%A6%E3%83%BB%E5%AE%8C%E4%BA%86%E6%9D%A1%E4%BB%B6%E3%83%BB%E4%BE%9D%E5%AD%98%E9%96%A2%E4%BF%82%3A%0Ahttps%3A%2F%2Fgithub.com%2FFuelSurcharge%2FOSS-CodeBot%2Fblob%2Fmain%2Fdocs%2Ftasks%2FCB-009.md) |
| [CB-010](CB-010.md) | [作成画面](https://github.com/FuelSurcharge/OSS-CodeBot/issues/new?title=%5BCB-010%5D+%E3%82%B3%E3%83%BC%E3%83%89%E5%A4%89%E6%9B%B4%E3%81%AE%E5%B7%AE%E5%88%86%E3%83%97%E3%83%AC%E3%83%93%E3%83%A5%E3%83%BC%E3%83%BB%E9%81%B8%E6%8A%9E%E9%81%A9%E7%94%A8%E3%83%BB%E5%8F%96%E3%82%8A%E6%B6%88%E3%81%97%E3%82%92%E8%BF%BD%E5%8A%A0%E3%81%99%E3%82%8B&body=%3C%21--+codebot-task%3ACB-010+--%3E%0A%0A%E4%BB%95%E6%A7%98%E3%83%BB%E5%84%AA%E5%85%88%E5%BA%A6%E3%83%BB%E5%AE%8C%E4%BA%86%E6%9D%A1%E4%BB%B6%E3%83%BB%E4%BE%9D%E5%AD%98%E9%96%A2%E4%BF%82%3A%0Ahttps%3A%2F%2Fgithub.com%2FFuelSurcharge%2FOSS-CodeBot%2Fblob%2Fmain%2Fdocs%2Ftasks%2FCB-010.md) |
| [CB-011](CB-011.md) | [作成画面](https://github.com/FuelSurcharge/OSS-CodeBot/issues/new?title=%5BCB-011%5D+%E7%A2%BA%E8%AA%8D%E4%BB%98%E3%81%8D%E3%81%AE%E3%82%B3%E3%83%9E%E3%83%B3%E3%83%89%E5%AE%9F%E8%A1%8C%E3%81%A8%E3%83%86%E3%82%B9%E3%83%88%E7%B5%90%E6%9E%9C%E8%A1%A8%E7%A4%BA%E3%82%92%E8%BF%BD%E5%8A%A0%E3%81%99%E3%82%8B&body=%3C%21--+codebot-task%3ACB-011+--%3E%0A%0A%E4%BB%95%E6%A7%98%E3%83%BB%E5%84%AA%E5%85%88%E5%BA%A6%E3%83%BB%E5%AE%8C%E4%BA%86%E6%9D%A1%E4%BB%B6%E3%83%BB%E4%BE%9D%E5%AD%98%E9%96%A2%E4%BF%82%3A%0Ahttps%3A%2F%2Fgithub.com%2FFuelSurcharge%2FOSS-CodeBot%2Fblob%2Fmain%2Fdocs%2Ftasks%2FCB-011.md) |
| [CB-012](CB-012.md) | [作成画面](https://github.com/FuelSurcharge/OSS-CodeBot/issues/new?title=%5BCB-012%5D+Git%E3%81%AE%E7%8A%B6%E6%85%8B%E3%83%BB%E5%B7%AE%E5%88%86%E3%83%BB%E3%83%96%E3%83%A9%E3%83%B3%E3%83%81%E3%83%BB%E3%82%B3%E3%83%9F%E3%83%83%E3%83%88%E6%93%8D%E4%BD%9C%E3%82%92%E8%BF%BD%E5%8A%A0%E3%81%99%E3%82%8B&body=%3C%21--+codebot-task%3ACB-012+--%3E%0A%0A%E4%BB%95%E6%A7%98%E3%83%BB%E5%84%AA%E5%85%88%E5%BA%A6%E3%83%BB%E5%AE%8C%E4%BA%86%E6%9D%A1%E4%BB%B6%E3%83%BB%E4%BE%9D%E5%AD%98%E9%96%A2%E4%BF%82%3A%0Ahttps%3A%2F%2Fgithub.com%2FFuelSurcharge%2FOSS-CodeBot%2Fblob%2Fmain%2Fdocs%2Ftasks%2FCB-012.md) |
| [CB-013](CB-013.md) | [作成画面](https://github.com/FuelSurcharge/OSS-CodeBot/issues/new?title=%5BCB-013%5D+%E3%82%A2%E3%83%97%E3%83%AA%E3%81%8B%E3%82%89GitHub+Issue%E3%83%BBPull+Request%E3%82%92%E6%89%B1%E3%81%88%E3%82%8B%E3%82%88%E3%81%86%E3%81%AB%E3%81%99%E3%82%8B&body=%3C%21--+codebot-task%3ACB-013+--%3E%0A%0A%E4%BB%95%E6%A7%98%E3%83%BB%E5%84%AA%E5%85%88%E5%BA%A6%E3%83%BB%E5%AE%8C%E4%BA%86%E6%9D%A1%E4%BB%B6%E3%83%BB%E4%BE%9D%E5%AD%98%E9%96%A2%E4%BF%82%3A%0Ahttps%3A%2F%2Fgithub.com%2FFuelSurcharge%2FOSS-CodeBot%2Fblob%2Fmain%2Fdocs%2Ftasks%2FCB-013.md) |
| [CB-014](CB-014.md) | [作成画面](https://github.com/FuelSurcharge/OSS-CodeBot/issues/new?title=%5BCB-014%5D+API%E6%8E%A5%E7%B6%9A%E8%A8%BA%E6%96%AD%E3%81%A8%E5%88%A9%E7%94%A8%E5%8F%AF%E8%83%BD%E3%81%AA%E3%83%A2%E3%83%87%E3%83%AB%E4%B8%80%E8%A6%A7%E3%81%AE%E5%8F%96%E5%BE%97%E3%82%92%E8%BF%BD%E5%8A%A0%E3%81%99%E3%82%8B&body=%3C%21--+codebot-task%3ACB-014+--%3E%0A%0A%E4%BB%95%E6%A7%98%E3%83%BB%E5%84%AA%E5%85%88%E5%BA%A6%E3%83%BB%E5%AE%8C%E4%BA%86%E6%9D%A1%E4%BB%B6%E3%83%BB%E4%BE%9D%E5%AD%98%E9%96%A2%E4%BF%82%3A%0Ahttps%3A%2F%2Fgithub.com%2FFuelSurcharge%2FOSS-CodeBot%2Fblob%2Fmain%2Fdocs%2Ftasks%2FCB-014.md) |
| [CB-015](CB-015.md) | [作成画面](https://github.com/FuelSurcharge/OSS-CodeBot/issues/new?title=%5BCB-015%5D+Responses+API%E3%81%A8%E5%85%B1%E9%80%9A%E3%83%84%E3%83%BC%E3%83%AB%E5%91%BC%E3%81%B3%E5%87%BA%E3%81%97%E3%82%92%E8%BF%BD%E5%8A%A0%E3%81%99%E3%82%8B&body=%3C%21--+codebot-task%3ACB-015+--%3E%0A%0A%E4%BB%95%E6%A7%98%E3%83%BB%E5%84%AA%E5%85%88%E5%BA%A6%E3%83%BB%E5%AE%8C%E4%BA%86%E6%9D%A1%E4%BB%B6%E3%83%BB%E4%BE%9D%E5%AD%98%E9%96%A2%E4%BF%82%3A%0Ahttps%3A%2F%2Fgithub.com%2FFuelSurcharge%2FOSS-CodeBot%2Fblob%2Fmain%2Fdocs%2Ftasks%2FCB-015.md) |
| [CB-016](CB-016.md) | [作成画面](https://github.com/FuelSurcharge/OSS-CodeBot/issues/new?title=%5BCB-016%5D+Windows%E3%83%BBmacOS%E3%83%BBLinux%E3%81%AEGUI%E6%93%8D%E4%BD%9C%E3%81%A8%E6%97%A5%E6%9C%AC%E8%AA%9E%E5%85%A5%E5%8A%9B%E3%82%92%E6%A4%9C%E8%A8%BC%E3%81%99%E3%82%8B&body=%3C%21--+codebot-task%3ACB-016+--%3E%0A%0A%E4%BB%95%E6%A7%98%E3%83%BB%E5%84%AA%E5%85%88%E5%BA%A6%E3%83%BB%E5%AE%8C%E4%BA%86%E6%9D%A1%E4%BB%B6%E3%83%BB%E4%BE%9D%E5%AD%98%E9%96%A2%E4%BF%82%3A%0Ahttps%3A%2F%2Fgithub.com%2FFuelSurcharge%2FOSS-CodeBot%2Fblob%2Fmain%2Fdocs%2Ftasks%2FCB-016.md) |
| [CB-017](CB-017.md) | [作成画面](https://github.com/FuelSurcharge/OSS-CodeBot/issues/new?title=%5BCB-017%5D+3+OS%E5%90%91%E3%81%91%E3%81%AE%E9%85%8D%E5%B8%83%E3%83%91%E3%83%83%E3%82%B1%E3%83%BC%E3%82%B8%E3%81%A8%E3%83%AA%E3%83%AA%E3%83%BC%E3%82%B9%E6%89%8B%E9%A0%86%E3%82%92%E4%BD%9C%E3%82%8B&body=%3C%21--+codebot-task%3ACB-017+--%3E%0A%0A%E4%BB%95%E6%A7%98%E3%83%BB%E5%84%AA%E5%85%88%E5%BA%A6%E3%83%BB%E5%AE%8C%E4%BA%86%E6%9D%A1%E4%BB%B6%E3%83%BB%E4%BE%9D%E5%AD%98%E9%96%A2%E4%BF%82%3A%0Ahttps%3A%2F%2Fgithub.com%2FFuelSurcharge%2FOSS-CodeBot%2Fblob%2Fmain%2Fdocs%2Ftasks%2FCB-017.md) |
| [CB-018](CB-018.md) | [作成画面](https://github.com/FuelSurcharge/OSS-CodeBot/issues/new?title=%5BCB-018%5D+%E6%A8%A9%E9%99%90%E3%82%92%E6%98%8E%E7%A4%BA%E3%81%99%E3%82%8B%E3%83%97%E3%83%A9%E3%82%B0%E3%82%A4%E3%83%B3%E6%8B%A1%E5%BC%B5%E3%81%AE%E6%9C%80%E5%B0%8F%E4%BB%95%E6%A7%98%E3%82%92%E4%BD%9C%E3%82%8B&body=%3C%21--+codebot-task%3ACB-018+--%3E%0A%0A%E4%BB%95%E6%A7%98%E3%83%BB%E5%84%AA%E5%85%88%E5%BA%A6%E3%83%BB%E5%AE%8C%E4%BA%86%E6%9D%A1%E4%BB%B6%E3%83%BB%E4%BE%9D%E5%AD%98%E9%96%A2%E4%BF%82%3A%0Ahttps%3A%2F%2Fgithub.com%2FFuelSurcharge%2FOSS-CodeBot%2Fblob%2Fmain%2Fdocs%2Ftasks%2FCB-018.md) |
| [FUT-DRV-001](FUT-DRV-001.md) | [作成画面](https://github.com/FuelSurcharge/OSS-CodeBot/issues/new?title=%5BFUT-DRV-001%5D+%E6%9C%80%E5%88%9D%E3%81%AE%E7%8B%AC%E8%87%AA%E3%83%89%E3%83%A9%E3%82%A4%E3%83%90%E3%83%BC%E3%81%AE%E5%AF%BE%E8%B1%A1%E3%81%A8%E3%82%A4%E3%83%B3%E3%82%BF%E3%83%BC%E3%83%95%E3%82%A7%E3%83%BC%E3%82%B9%E3%82%92%E6%B1%BA%E3%82%81%E3%82%8B&body=%3C%21--+codebot-task%3AFUT-DRV-001+--%3E%0A%0A%E4%BB%95%E6%A7%98%E3%83%BB%E5%84%AA%E5%85%88%E5%BA%A6%E3%83%BB%E5%AE%8C%E4%BA%86%E6%9D%A1%E4%BB%B6%E3%83%BB%E4%BE%9D%E5%AD%98%E9%96%A2%E4%BF%82%3A%0Ahttps%3A%2F%2Fgithub.com%2FFuelSurcharge%2FOSS-CodeBot%2Fblob%2Fmain%2Fdocs%2Ftasks%2FFUT-DRV-001.md) |
| [FUT-OS-001](FUT-OS-001.md) | [作成画面](https://github.com/FuelSurcharge/OSS-CodeBot/issues/new?title=%5BFUT-OS-001%5D+%E7%8B%AC%E8%87%AAOS%E9%96%8B%E7%99%BA%E3%81%AE%E5%AF%BE%E8%B1%A1%E3%81%A8%E6%9C%80%E5%B0%8F%E8%B5%B7%E5%8B%95%E3%81%BE%E3%81%A7%E3%81%AE%E4%BB%95%E6%A7%98%E3%82%92%E6%B1%BA%E3%82%81%E3%82%8B&body=%3C%21--+codebot-task%3AFUT-OS-001+--%3E%0A%0A%E4%BB%95%E6%A7%98%E3%83%BB%E5%84%AA%E5%85%88%E5%BA%A6%E3%83%BB%E5%AE%8C%E4%BA%86%E6%9D%A1%E4%BB%B6%E3%83%BB%E4%BE%9D%E5%AD%98%E9%96%A2%E4%BF%82%3A%0Ahttps%3A%2F%2Fgithub.com%2FFuelSurcharge%2FOSS-CodeBot%2Fblob%2Fmain%2Fdocs%2Ftasks%2FFUT-OS-001.md) |

## APIによる一括登録

各タスクは本文に安定IDを持ち、優先度・現状・完了条件・依存関係を含む。
既存Issueを取得できた場合にだけ登録を始め、同じIDの既存Issueは閉じているものも再作成しない。
既存Issueの内容・状態・担当者・ラベルは変更しない。

```sh
# オフラインで内容と件数を確認（変更なし）
python3 scripts/create_github_issues.py

# GitHub APIへ接続できる環境で、現在のgh認証を使って登録
python3 scripts/create_github_issues.py --apply
```

作成時には依存タスクのIssue URLを本文へ追加する。API要求を自動再試行しないため、通信失敗時は既存Issueを確認してから再実行する。
タスク一覧を読めない場合は登録を中断する。Issue登録そのものの成功は表示された実URLで確認する。

## 現在の登録状況

GitHub API（api.github.com）への接続がHTTP 403で拒否されており、Issue登録は未実施。
通信先追加をクラウド環境のドラフトに保存済み。環境設定で確認・保存して公開後、接続確認と重複確認を行って登録を再開する。

## 参照

- [初期設計書](../OSS_CodeBot_DESIGN.md)
- [現在のGUI方針](../../DESIGN.md)
- [ローカルモデルの実装と制限](../LOCAL_MODELS.md)
- [APIの実装と制限](../API.md)
- [独自OS・ドライバーの既存ロードマップ](../../OSS_CodeBot_ROADMAP.md)
