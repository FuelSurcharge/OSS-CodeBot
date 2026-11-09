# Issue・開発タスク一覧

最終更新: 2026-11-09（日本時間）

開発タスクはこのファイルと `docs/tasks/` のMarkdownで管理する。
全20件中、完了14件・残り6件（アプリ4件、将来構想2件）。
各リンク先に現状・完了条件・依存関係・検証結果を記載する。
`CB-003`などはファイル管理用のIDで、GitHubのIssue番号とは異なる。

## P0：次に進める機能

- [ ] [CB-005：ローカル推論の診断・初回セットアップ](docs/tasks/CB-005.md) — 実装済み。Linux検証済み、Windows/macOSの独立環境での確認待ち。
- [ ] [CB-006：公式配布元・互換性を確認したモデルカタログ](docs/tasks/CB-006.md)
- [ ] [CB-016：Windows・macOS・LinuxのGUI・日本語入力検証](docs/tasks/CB-016.md) — Linuxの主要機能を確認済み。検証表を追加、他OS/IME/高DPIなどは未確認。

## P1：開発支援と運用の拡張

- [ ] [CB-017：3 OS向け配布パッケージ・リリース手順](docs/tasks/CB-017.md) — 生成処理・4target CI・手順を実装。Linux版を展開して起動確認。本体ライセンス、他OS/新規環境検証が残る。

## 将来構想：別プロジェクト

- [ ] [FUT-OS-001：独自OSの仕様・最小起動までのタスク分割](docs/tasks/FUT-OS-001.md)
- [ ] [FUT-DRV-001：独自ドライバーの対象・インターフェース策定](docs/tasks/FUT-DRV-001.md)

## 完了

- [x] [CB-018：権限を明示するプラグイン拡張](docs/tasks/CB-018.md) — 2026-11-09。実装・Wasm制限と権限の自動検証・Linux実GUI/CLIで導入/実行/削除と確認付き変更を検証完了。他OS実画面は未検証。
- [x] [CB-015：Responses API・共通ツール呼び出し](docs/tasks/CB-015.md) — 2026-11-09。実装・4形式の模擬APIとLinux実GUI/CLI検証完了。各社実APIのモデル互換性は未検証。
- [x] [CB-007：モデル取得再開・分割GGUF・キャッシュ管理](docs/tasks/CB-007.md) — 2026-11-09。実装・模擬HTTPとLinux実GUI/CLI検証完了。Hugging Face実通信・学習済み分割GGUF推論は未検証。
- [x] [CB-013：アプリからGitHub Issue・Pull Requestを扱う](docs/tasks/CB-013.md) — 2026-11-09。実装・模擬APIとLinux実GUI検証完了。本番GitHub認証・投稿は未検証。
- [x] [CB-012：Gitの状態・差分・ブランチ・コミット](docs/tasks/CB-012.md) — 2026-11-09。実装・Linux実Git/GUI検証完了。ローカルbareへのpush確認、GitHub認証と他OS実機は未検証。
- [x] [CB-011：確認付きコマンド実行・テスト結果表示](docs/tasks/CB-011.md) — 2026-11-09。実装・Linux検証完了。他OSの実機検証はCB-016。
- [x] [CB-014：API接続診断・利用可能なモデル一覧](docs/tasks/CB-014.md) — 2026-11-09。実装・模擬API検証完了。各社の実API認証は未検証と記録。
- [x] [CB-002：ストリーミング表示・生成停止](docs/tasks/CB-002.md) — 2026-11-09。実装・検証完了。
- [x] [CB-010：変更差分の確認・選択適用・取り消し](docs/tasks/CB-010.md) — 2026-11-09。実装・検証完了。
- [x] [CB-001：会話履歴を含む継続対話・CLI対話モード](docs/tasks/CB-001.md) — 2026-11-09。実装・検証完了。
- [x] [CB-004：チャット保存・再開・名前変更・削除・書き出し](docs/tasks/CB-004.md) — 2026-11-09。実装・検証完了。
- [x] [CB-008：作業フォルダ・ファイル読み込み](docs/tasks/CB-008.md) — 2026-11-09。実装・検証完了。
- [x] [CB-009：ファイルを使うコード説明・ログ解析・レビュー](docs/tasks/CB-009.md) — 2026-11-09。実装・検証完了。

- [x] [CB-003：Markdown・コードブロック表示・コピー](docs/tasks/CB-003.md) — 2026-11-09。[実装コミット f089a62](https://github.com/FuelSurcharge/OSS-CodeBot/commit/f089a62)。Rustテスト32件・ビルド成功、Linuxの通常サイズと700×540で表示確認。

## 既存GitHub Issueとの対応

以下の番号と件名は、利用者が提示したGitHubの一覧に基づく。
GitHub Issue本体の最新状態は未取得。この表は関連する作業の対応表であり、完了・クローズを意味しない。

| GitHub Issue | 内容 | 関連するファイル管理タスク |
|---|---|---|
| [#2](https://github.com/FuelSurcharge/OSS-CodeBot/issues/2) | 初期アーキテクチャの定義 | 初期設計書と既存のRust共通コア・CLI・GUI。Issue本文の完了条件は未確認 |
| [#3](https://github.com/FuelSurcharge/OSS-CodeBot/issues/3) | ソースコード生成・修正 | CB-009、CB-010 |
| [#4](https://github.com/FuelSurcharge/OSS-CodeBot/issues/4) | GitHubリポジトリ連携 | CB-012、CB-013 |
| [#5](https://github.com/FuelSurcharge/OSS-CodeBot/issues/5) | 自動コードレビュー・不具合検出 | CB-009 |
| [#6](https://github.com/FuelSurcharge/OSS-CodeBot/issues/6) | 独自OS開発 | FUT-OS-001 |
| [#7](https://github.com/FuelSurcharge/OSS-CodeBot/issues/7) | 独自ドライバー開発 | FUT-DRV-001 |

CB-003は上記6件のいずれかを単独で解決する変更ではない。

## 更新方法

1. 着手時にリンク先の「状態」を開発中へ変更する。
2. 実装・検証した完了条件にチェックし、実施した検証と未検証の範囲を記録する。
3. 完了したタスクをこの一覧の「完了」へ移し、件数と日付を更新する。
4. ファイルの変更をコミットしてpushする。GitHub Issueの状態変更は別途行った場合だけ記録する。

関連文書: [初期設計](docs/OSS_CodeBot_DESIGN.md)、[GUI方針](DESIGN.md)、[独自OS・ドライバーのロードマップ](OSS_CodeBot_ROADMAP.md)。
