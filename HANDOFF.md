# 引き継ぎプロンプト（/clear 後にこれを貼る）

```text
~/work/ai/rain-alert の続きをやります。まず WORKLOG.md の冒頭「現在地」と末尾のログ、PLAN.md を読んで状況をつかんでください（BRIEF.md・request.png は元の依頼資料で、リポジトリには入れない私的ファイル）。

■ これは何
自宅まわりで雨が降り始めたら止む予定時刻つき、止んだら次に降り出す予定時刻つきで、Discord「EngineMakerβ版」の #em新宿_雑談_chat（Webhook 名「雨のお知らせ」）に通知する。リビングの電光掲示板（signboard）にも、雨の間と30分以内に降り出しそうな間だけ表示する。見るのはシェアハウスの住人。
- Rust 製。em105 の systemd --user timer（deploy/rain-alert.{service,timer}、~/.config/systemd/user にシンボリックリンク）で5分ごとに1回起動して終了
- 60分先までは Yahoo! 気象情報 API（5分刻み）、それより先は気象庁 降水短時間予報のタイルの色を読んで「〇時ごろ」の目安
- リポジトリ: https://github.com/EngineMaker/rain-alert（public）。.env（appid・Webhook URL・緯度経度・signboard キー）と data/ は git 管理外。.env の値は読まない・表示しない

■ いまの段階
段階3（本番運用中、2026-09-30〜）。通知先は #em新宿_雑談_chat。#sandbox での試運転は良好だった（最初の「止みました」は 2026-09-30 05:07、誤通知なし）。
Cosense ページは https://scrapbox.io/EngineMaker/雨のお知らせ（旧題「雨のお告げ」）。編集方法はメモリ cosense-edit-via-agent-browser を参照。書く前に最新を読んでユーザーの手直しをマージし、書いた後に完全一致を確認。docs/cosense.txt もページと同じにする
次にやること: 本番での住人の反応を見て、必要なら文面・しきい値を調整

■ ファイル
- src/main.rs（全体の流れ）/ config.rs（.env と環境変数）/ yahoo.rs（Yahoo! API）/ judge.rs（雨・雨なしの判定と Discord の文面）/ jma.rs（気象庁の目安）/ signboard.rs（掲示板）
- deploy/rain-alert.service・rain-alert.timer、.env.example、README.md、PLAN.md、WORKLOG.md、docs/cosense.txt、assets/icon.{png,svg}
- 実行時データ: data/state.json（雨の状態）、data/signboard.json（掲示板に出しているお知らせ）、data/jma-cache.json、data/log.jsonl（毎回の取得値と判定）

■ コマンド
- テスト: ~/.cargo/bin/cargo test（20件）
- ビルド: ~/.cargo/bin/cargo build --release（timer は target/release/rain-alert を直接起動するので、ビルドすれば次の回から反映）
- 手動で1回: cd ~/work/ai/rain-alert && ./target/release/rain-alert
- 様子: systemctl --user list-timers rain-alert.timer / journalctl --user -u rain-alert.service / tail data/log.jsonl

■ 関係者
- signboard の現場セッション（名前 signboard）: 掲示板 API の相談先。API は https://signboard.emaker.dev/api/v1、404 は code（not_found / deleted / expired）で判断
- em105-HQ: 進み具合の報告先

■ 進め方の約束
- 回答は日本語。最後に「あなたのアクション」と「確認したいこと」をまとめる
- 新しい機能は、いきなり実装せず計画を出して了承をもらってから
- 説明に内部の記号や変数名を使わず、中身を平易な言葉で言う
- 経緯は WORKLOG.md に追記し、冒頭の「現在地」も更新。コミット・push まで行う
```
