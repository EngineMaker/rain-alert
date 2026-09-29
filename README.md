# rain-alert

指定した地点で雨が

- **降り始めたら** → 止む予定時刻つきで
- **止んだら** → 次に降り始める予定時刻つきで

Discord の Webhook に通知する小さなツール。

```
☔ 自宅まわりで雨が降ってきました（弱い雨）。22:40ごろ止む見込みです。
🌤 自宅まわりの雨が止みました。1時間以内は降らない見込みです。
```

## しくみ

- [Yahoo! 気象情報 API](https://developer.yahoo.co.jp/webapi/map/openlocalplatform/v1/weather.html) から、地点の降水強度の実測と 60 分先までの予測（5 分刻み）を取る
- 実測が「1 mm/h 以上 × 2 回連続」で降り始め、「0.5 mm/h 未満 × 3 回連続」で止みと判定。状態が変わってから 30 分は逆向きに変えない（ぱらつきで通知が連発しないように）
- 予定時刻は予測の中で最初に切り替わる時刻。**予測は 60 分先までしかない**ので、それより先は「1時間以内には止まない見込み」などと出す
- 常駐せず、systemd timer で 5 分ごとに 1 回起動して終了する（Rust 製、バイナリ約 1.7MB、実行時メモリ約 4MB）

## 使い方

```sh
cp .env.example .env   # appid・Webhook URL・緯度経度を記入
cargo build --release
./target/release/rain-alert   # 1 回判定して終了。DRY_RUN=1 なら送信せず表示だけ
```

状態は `data/state.json`、毎回の取得値と判定は `data/log.jsonl` に残る。

### systemd（ユーザーサービス）で 5 分ごとに動かす

```sh
ln -sf $PWD/deploy/rain-alert.service $PWD/deploy/rain-alert.timer ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now rain-alert.timer
journalctl --user -u rain-alert.service   # ログ
```

`deploy/rain-alert.service` はリポジトリが `~/work/ai/rain-alert` にある前提。

## クレジット

気象情報: Web Services by Yahoo! JAPAN
