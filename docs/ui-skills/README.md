# UI Skillsの適用

出典: https://www.ui-skills.com/ / https://github.com/ibelick/ui-skills
参照コミット: 73fb128d046d1f0d74c2909cd26eb616e3415567

このディレクトリのSkill本文は上流から取得した参照用コピーです（MIT、LICENSE同梱）。
今後のGUI変更では baseline-ui.md と fixing-accessibility.md を読み、Rust/eguiに適用可能な指針を使ってください。
Web専用のTailwind・React・ARIA属性指定は直接適用せず、egui標準部品とラベル関連付けを用います。

今回の適用:
- 手描きのクリック領域を標準Buttonに変更し、キーボード操作・名前を提供。
- 入力欄と設定欄に可視ラベルを関連付け。
- 送信ボタンに可視の操作名を追加、無効状態の理由を表示。
- 入力例はユーザー要望により削除。
- 設定はEscapeで閉じる。設定操作中の送信ショートカットを抑止。

スクリーンリーダー、各OSのIME、全操作のフォーカス遷移の検証は未完了です。
Webメタデータやアニメーション性能のSkillは、この変更には適用していません。

## 2026-11-04: frontend-designを使った再設計

追加で参照したSkill:
https://github.com/anthropics/skills/blob/main/skills/frontend-design/SKILL.md

本文を取得して読み、設計計画→実装→実画面による批評の手順を適用。
このSkillの本文は本リポジトリには再配布せず、上記リンクを参照する。

### 設計計画と見直し

対象は、ローカルでログを調査する開発者向けデスクトップツール。
広告的な大見出し、英語の装飾ラベル、紫のブランド色、反復カードを外す。
装飾を別の流行色に置き換える案は採らず、ログ入力と診断結果を主役にする。

- 作業面: SURFACE #FFFFFF、サイドバー: SIDEBAR #F3F4F6、入力面: INPUT #F8F9FA。
- 文字: INK #252A31、補足: MUTED #646C76、選択: SELECTION #E1E9F2。
- 日本語本文はNoto Sans JP Regular。本文14px、補助11–13px、見出し18px。
- 上部ツールバー、左にセッション、中央に診断履歴、下に入力欄と実行操作を配置。
- UIは左から読む順序を基本とし、実行ボタンを入力欄右下、設定を右上に置く。
- 空状態は次の操作を短く説明する。実装していない機能をナビゲーションに並べない。
- アニメーション・装飾アイコン・販促コピーは追加しない。

### 確認

Linux仮想ディスプレイで1100×760と760×540を確認。
入力、Ctrl+Enterによる診断、結果表示を確認。
小さい画面では履歴をスクロールし、入力欄と診断ボタンは表示を維持する。
Windows/macOS、スクリーンリーダーの操作検証は未実施。

## WinUI 3参照

ユーザー指定により公式WinUI Galleryの以下の実装を確認:
- https://github.com/microsoft/WinUI-Gallery/blob/main/WinUIGallery/Samples/NavigationView/NavigationViewPage.xaml
- https://github.com/microsoft/WinUI-Gallery/blob/main/WinUIGallery/Samples/Button/ButtonPage.xaml
- https://github.com/microsoft/WinUI-Gallery/blob/main/WinUIGallery/Samples/TextBox/TextBoxPage.xaml

左ナビゲーションの選択インジケーターと下部の設定、アクセント付き実行ボタン、ラベル付き入力をeguiで表現。
入力例は削除。実装はWinUI 3ネイティブではなく、全OS共通のRust/egui。
MicaやWindows固有のウィンドウ効果は実装していない。

## 会話中心の再設計

最新の基準はルートのDESIGN.md。上記の配色・レイアウトは過去の検討記録です。

追加で取得・参照したImpeccable:
https://github.com/pbakaus/impeccable/blob/f864521bbb1c9350e8f6cf7742c139a04a1020af/.agents/skills/impeccable/SKILL.md

同リポジトリの reference/operate.md と reference/craft-floor.md を読み、操作用UIの情報階層、読める幅、部品の一貫性、実画面によるレビューを適用しました。
付属contextコマンドは既定キャッシュに書き込めず実行不能だったため、既存コードと設計書を直接確認しました。SkillのCLI全体・コンセプト選択サービスは導入しておらず、フルワークフロー完了とは主張しません。

独立レビューで旧構成の問題と状態管理を確認。新規のGUIテスト4件を追加しました。
区切り線で5分割したフォームを廃止し、最大680pxの会話と入力を同軸に配置。設定を同じ画面内に移し、入力例は削除を維持しました。
