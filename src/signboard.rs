//! リビングの電光掲示板（signboard）に、雨が降っている間と、もうすぐ降りそうな間だけお知らせを出す。
//! API: https://signboard.emaker.dev/api/v1 （EngineMaker/signboard の docs/API.md）
//!
//! POST するたびに掲示板で 5 分間光るので、POST は雨ごとに 1 回だけにして、あとは PATCH で期限を延ばす。
//! 条件が外れたら何もしない（期限が来て自然に消える）。手で消されたら、その雨の間はもう出さない。
//! PATCH の 404 は理由の code で分ける（signboard D-037）: deleted（手で消された）なら出さない、expired・not_found なら出し直す。

use serde::{Deserialize, Serialize};

use crate::judge::{self, Rules};
use crate::yahoo::Series;

pub struct Settings {
    pub base_url: String,
    pub api_key: String,
    /// この分数以内に降り出す予報なら表示する
    pub lead_min: i64,
    /// 表示の有効期限 [分]
    pub hold_min: i64,
}

/// 掲示板に出しているお知らせ。条件が外れたら捨てる（次の雨では新しく POST して光らせる）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Board {
    pub id: u64,
    pub body: String,
    pub expires_at_ms: i64,
    /// 誰かが手で消した。この雨の間はもう出さない
    #[serde(default)]
    pub suppressed: bool,
}

/// 何をするか（通信の前に決める。テストしやすくするため）
#[derive(Debug, PartialEq)]
pub enum Action {
    None,
    Forget,
    Post { body: String, expires_at_ms: i64 },
    Patch { id: u64, body: Option<String>, expires_at_ms: i64 },
}

/// 表示したい本文。表示しないなら None
pub fn wanted_body(raining: bool, s: &Series, r: &Rules, lead_min: i64) -> Option<String> {
    if raining {
        let now_mmh = s.observed.last().map_or(0.0, |p| p.mmh);
        let stop = judge::settled_from(&s.forecast, |v| v < r.stop_mmh);
        // 掲示板は横に流れて一度に全文が見えないので、大事なこと（いつまで降るか）を先頭に置く
        let strength = judge::strength(now_mmh);
        return Some(match stop {
            Some(t) => format!("☔ {}ごろまで雨（いま{strength}）", judge::hhmm(&t)),
            None => format!("☔ 雨がしばらく降り続けます（いま{strength}）"),
        });
    }
    let now = judge::minutes(&s.now);
    s.forecast
        .iter()
        .find(|p| p.mmh >= r.start_mmh && judge::minutes(&p.time) - now <= lead_min)
        .map(|p| format!("🌂 {}ごろ雨が降り出しそう", judge::hhmm(&p.time)))
}

pub fn plan(board: Option<&Board>, want: Option<String>, now_ms: i64, hold_min: i64) -> Action {
    let hold_ms = hold_min * 60_000;
    let expires_at_ms = now_ms + hold_ms;
    match (board, want) {
        (None, None) => Action::None,
        (Some(_), None) => Action::Forget,
        (None, Some(body)) => Action::Post { body, expires_at_ms },
        (Some(b), Some(_)) if b.suppressed => Action::None,
        // 障害などで延長が間に合わず期限が切れていたら、PATCH は 404（期限切れ）になるので新しく出し直す。
        // 手で消されたときの 404 と取り違えないよう、PATCH する前に自分で判定する（時計のずれに 30 秒の余裕）
        (Some(b), Some(body)) if b.expires_at_ms - now_ms < 30_000 => Action::Post { body, expires_at_ms },
        (Some(b), Some(body)) => {
            let changed = b.body != body;
            // 監査ログを増やしすぎないよう、残りが半分を切ったときか本文が変わったときだけ延ばす
            let running_low = b.expires_at_ms - now_ms < hold_ms / 2;
            if changed || running_low {
                Action::Patch { id: b.id, body: changed.then_some(body), expires_at_ms }
            } else {
                Action::None
            }
        }
    }
}

/// Action を実行して、次に覚えておく Board を返す
pub fn apply(cfg: &Settings, board: Option<&Board>, action: Action, dry_run: bool) -> Result<Option<Board>, String> {
    match action {
        Action::None => Ok(board.cloned()),
        Action::Forget => Ok(None),
        Action::Post { body, expires_at_ms } => post(cfg, body, expires_at_ms, dry_run),
        Action::Patch { id, body, expires_at_ms } => {
            let prev = board.cloned().ok_or("PATCH なのに Board がない")?;
            if dry_run {
                println!("[DRY_RUN] signboard PATCH #{id}: {body:?}");
                return Ok(Some(prev));
            }
            let mut payload = serde_json::json!({ "expiresAt": expires_at_ms });
            if let Some(b) = &body {
                payload["body"] = serde_json::json!(b);
            }
            match send(cfg, "PATCH", &format!("/notices/{id}"), &payload)? {
                Reply::Ok(_) => {
                    println!("[SIGNBOARD] PATCH #{id}");
                    Ok(Some(Board { id, body: body.unwrap_or(prev.body), expires_at_ms, suppressed: false }))
                }
                // 手で消された → この雨の間はもう出さない
                Reply::NotFound(code) if code == "deleted" => {
                    println!("[SIGNBOARD] #{id} は消されていたので、この雨の間は出さない");
                    Ok(Some(Board { suppressed: true, ..prev }))
                }
                // 期限切れ（expired）や、ID が無い（not_found）→ 新しく出し直す
                Reply::NotFound(code) => {
                    println!("[SIGNBOARD] #{id} は {code} なので出し直す");
                    post(cfg, body.unwrap_or(prev.body), expires_at_ms, false)
                }
            }
        }
    }
}

fn post(cfg: &Settings, body: String, expires_at_ms: i64, dry_run: bool) -> Result<Option<Board>, String> {
    if dry_run {
        println!("[DRY_RUN] signboard POST: {body}");
        return Ok(None);
    }
    let payload = serde_json::json!({ "body": body, "expiresAt": expires_at_ms });
    let Reply::Ok(res) = send(cfg, "POST", "/notices", &payload)? else {
        return Err("signboard の POST が 404".into());
    };
    let id = res["notice"]["id"].as_u64().ok_or("signboard の POST 応答に notice.id がない")?;
    println!("[SIGNBOARD] POST #{id}: {body}");
    Ok(Some(Board { id, body, expires_at_ms, suppressed: false }))
}

enum Reply {
    Ok(serde_json::Value),
    /// 404。中身は理由の code（not_found / deleted / expired。signboard D-037）
    NotFound(String),
}

/// 2xx と 404 以外は Err
fn send(cfg: &Settings, method: &str, path: &str, body: &serde_json::Value) -> Result<Reply, String> {
    let url = format!("{}{path}", cfg.base_url.trim_end_matches('/'));
    let auth = format!("Bearer {}", cfg.api_key);
    // 404 の本文（code）を読むため、エラー応答も Ok で受け取る
    let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).build().into();
    let req = match method {
        "POST" => agent.post(&url),
        _ => agent.patch(&url),
    };
    let mut res = req
        .header("Authorization", &auth)
        .header("Content-Type", "application/json")
        .send(body.to_string())
        .map_err(|e| format!("signboard {method} {path} に失敗: {e}"))?;
    let status = res.status().as_u16();
    let text = res.body_mut().read_to_string().map_err(|e| format!("signboard の応答を読めない: {e}"))?;
    let json: serde_json::Value = serde_json::from_str(&text).unwrap_or(serde_json::Value::Null);
    match status {
        200..=299 => Ok(Reply::Ok(json)),
        404 => Ok(Reply::NotFound(json["code"].as_str().unwrap_or("not_found").to_string())),
        _ => Err(format!("signboard {method} {path} が {status}: {}", json["error"].as_str().unwrap_or(&text))),
    }
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::yahoo::Point;

    fn rules() -> Rules {
        Rules { start_mmh: 1.0, start_count: 2, stop_mmh: 0.5, stop_count: 3, cooldown_min: 30 }
    }

    /// 22:10 が最新の実測で、予測は 22:15 から 5 分刻み
    fn series(now_mmh: f64, fc: &[f64]) -> Series {
        let t = |m: usize| format!("20260929{:02}{:02}", m / 60, m % 60);
        let base = 22 * 60 + 10;
        Series {
            now: t(base),
            observed: vec![Point { time: t(base), mmh: now_mmh }],
            forecast: fc.iter().enumerate().map(|(i, &v)| Point { time: t(base + 5 * (i + 1)), mmh: v }).collect(),
        }
    }

    fn board(body: &str, expires_at_ms: i64) -> Board {
        Board { id: 7, body: body.into(), expires_at_ms, suppressed: false }
    }

    #[test]
    fn body_while_raining() {
        let b = wanted_body(true, &series(1.5, &[1.0, 0.3, 0.0]), &rules(), 30).unwrap();
        assert_eq!(b, "☔ 22:20ごろまで雨（いま弱い雨）");
        let b = wanted_body(true, &series(1.5, &[1.0, 0.3, 2.0]), &rules(), 30).unwrap();
        assert_eq!(b, "☔ 雨がしばらく降り続けます（いま弱い雨）");
    }

    #[test]
    fn body_when_rain_is_near() {
        // 22:35 に降り出す予測（25 分後）→ 表示
        let s = series(0.0, &[0.0, 0.0, 0.0, 0.0, 2.0]);
        assert_eq!(wanted_body(false, &s, &rules(), 30).unwrap(), "🌂 22:35ごろ雨が降り出しそう");
        // 30 分より先なら表示しない
        let s = series(0.0, &[0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 2.0]);
        assert_eq!(wanted_body(false, &s, &rules(), 30), None);
    }

    #[test]
    fn posts_once_then_extends_only_when_needed() {
        let now = 1_000_000;
        let hold = 20 * 60_000;
        assert_eq!(plan(None, Some("a".into()), now, 20), Action::Post { body: "a".into(), expires_at_ms: now + hold });
        // 残りが十分・本文も同じ → 何もしない
        assert_eq!(plan(Some(&board("a", now + hold - 5 * 60_000)), Some("a".into()), now, 20), Action::None);
        // 残りが半分を切った → 期限だけ延長
        assert_eq!(
            plan(Some(&board("a", now + 9 * 60_000)), Some("a".into()), now, 20),
            Action::Patch { id: 7, body: None, expires_at_ms: now + hold }
        );
        // 本文が変わった → 本文も送る
        assert_eq!(
            plan(Some(&board("a", now + hold)), Some("b".into()), now, 20),
            Action::Patch { id: 7, body: Some("b".into()), expires_at_ms: now + hold }
        );
    }

    #[test]
    fn reposts_when_our_notice_has_expired() {
        let now = 1_000_000;
        assert_eq!(
            plan(Some(&board("a", now - 60_000)), Some("a".into()), now, 20),
            Action::Post { body: "a".into(), expires_at_ms: now + 20 * 60_000 }
        );
    }

    #[test]
    fn forgets_when_condition_ends_and_respects_manual_delete() {
        assert_eq!(plan(Some(&board("a", 0)), None, 0, 20), Action::Forget);
        let suppressed = Board { suppressed: true, ..board("a", 0) };
        assert_eq!(plan(Some(&suppressed), Some("b".into()), 0, 20), Action::None);
    }
}
