//! rain-alert: 雨の降り始め・止みを予定時刻つきで Discord に通知する。
//! systemd timer から 5 分ごとに 1 回起動され、判定して終了するワンショット実行。

mod config;
mod judge;
mod signboard;
mod yahoo;

use std::fs;
use std::io::Write;
use std::path::Path;

use config::Config;
use judge::{Event, State};

fn main() {
    if let Err(e) = run() {
        eprintln!("rain-alert: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let cfg = Config::load()?;
    let series = yahoo::fetch(&cfg)?;

    let state_path = Path::new(&cfg.data_dir).join("state.json");
    let prev = load_state(&state_path);
    let (next, event) = judge::decide(prev.as_ref(), &series, &cfg.rules);

    let message = event.as_ref().map(|ev| judge::message(ev, &cfg.label));
    if let Some(msg) = &message {
        if cfg.dry_run {
            println!("[DRY_RUN] {msg}");
        } else {
            post_discord(&cfg.webhook_url, msg)?;
            println!("[SENT] {msg}");
        }
    }

    fs::create_dir_all(&cfg.data_dir).map_err(|e| format!("データ用ディレクトリを作れない: {e}"))?;
    if let Some(sb) = &cfg.signboard {
        // 掲示板の失敗で Discord 側の状態保存まで止めない
        if let Err(e) = update_signboard(sb, &cfg, &series, next.raining) {
            eprintln!("rain-alert: {e}");
        }
    }
    append_log(&cfg.data_dir, &series, &next, event.as_ref(), message.as_deref(), cfg.dry_run)?;
    save_state(&state_path, &next)
}

fn update_signboard(sb: &signboard::Settings, cfg: &Config, series: &yahoo::Series, raining: bool) -> Result<(), String> {
    let path = Path::new(&cfg.data_dir).join("signboard.json");
    let board: Option<signboard::Board> = fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str(&t).ok());
    let want = signboard::wanted_body(raining, series, &cfg.rules, sb.lead_min);
    let action = signboard::plan(board.as_ref(), want, signboard::now_ms(), sb.hold_min);
    let next = signboard::apply(sb, board.as_ref(), action, cfg.dry_run)?;
    match next {
        Some(b) => fs::write(&path, serde_json::to_string_pretty(&b).map_err(|e| e.to_string())?)
            .map_err(|e| format!("掲示板の状態を書けない: {e}")),
        None => match fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(format!("掲示板の状態を消せない: {e}")),
            _ => Ok(()),
        },
    }
}

fn load_state(path: &Path) -> Option<State> {
    let text = fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

fn save_state(path: &Path, state: &State) -> Result<(), String> {
    // 途中で落ちても壊れた state.json が残らないよう、一時ファイルに書いてから置き換える
    let tmp = path.with_extension("json.tmp");
    let text = serde_json::to_string_pretty(state).map_err(|e| e.to_string())?;
    fs::write(&tmp, text).map_err(|e| format!("状態を書けない: {e}"))?;
    fs::rename(&tmp, path).map_err(|e| format!("状態を置き換えられない: {e}"))
}

fn append_log(
    dir: &str,
    series: &yahoo::Series,
    state: &State,
    event: Option<&Event>,
    message: Option<&str>,
    dry_run: bool,
) -> Result<(), String> {
    let line = serde_json::json!({
        "now": series.now,
        "observed": series.observed,
        "forecast": series.forecast,
        "raining": state.raining,
        "event": event.map(|e| e.kind()),
        "message": message,
        "dry_run": dry_run,
    });
    let mut f = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(Path::new(dir).join("log.jsonl"))
        .map_err(|e| format!("ログを開けない: {e}"))?;
    writeln!(f, "{line}").map_err(|e| format!("ログを書けない: {e}"))
}

fn post_discord(webhook_url: &str, content: &str) -> Result<(), String> {
    let body = serde_json::json!({
        "content": content,
        // メンションを誤爆させない
        "allowed_mentions": { "parse": [] },
    });
    ureq::post(webhook_url)
        .header("Content-Type", "application/json")
        .send(body.to_string())
        .map_err(|e| format!("Discord への送信に失敗: {e}"))?;
    Ok(())
}
