//! 設定は環境変数から読む。手元で動かすときのために、カレントの .env も読む
//! （systemd では EnvironmentFile で渡すので、既に設定済みの変数は上書きしない）。

use std::env;
use std::fs;

use crate::judge::Rules;
use crate::signboard;

pub struct Config {
    pub appid: String,
    pub webhook_url: String,
    pub lat: String,
    pub lon: String,
    pub label: String,
    pub data_dir: String,
    pub dry_run: bool,
    pub rules: Rules,
    /// SIGNBOARD_API_KEY が空なら None（掲示板と連携しない）
    pub signboard: Option<signboard::Settings>,
}

impl Config {
    pub fn load() -> Result<Self, String> {
        load_dotenv(".env");
        let dry_run = opt("DRY_RUN", "1") != "0";
        Ok(Self {
            appid: required("YAHOO_APPID")?,
            // DRY_RUN のときは Webhook なしでも動かせる
            webhook_url: if dry_run { opt("DISCORD_WEBHOOK_URL", "") } else { required("DISCORD_WEBHOOK_URL")? },
            lat: required("LAT")?,
            lon: required("LON")?,
            label: opt("LOCATION_LABEL", "自宅まわり"),
            data_dir: opt("DATA_DIR", "data"),
            dry_run,
            rules: Rules {
                start_mmh: num("RAIN_START_MMH", 1.0)?,
                start_count: num("RAIN_START_COUNT", 2.0)? as usize,
                stop_mmh: num("RAIN_STOP_MMH", 0.5)?,
                stop_count: num("RAIN_STOP_COUNT", 3.0)? as usize,
                cooldown_min: num("COOLDOWN_MIN", 30.0)? as i64,
            },
            signboard: match opt("SIGNBOARD_API_KEY", "") {
                key if key.is_empty() => None,
                api_key => Some(signboard::Settings {
                    base_url: opt("SIGNBOARD_URL", "https://signboard.emaker.dev/api/v1"),
                    api_key,
                    lead_min: num("SIGNBOARD_LEAD_MIN", 30.0)? as i64,
                    hold_min: num("SIGNBOARD_HOLD_MIN", 20.0)? as i64,
                }),
            },
        })
    }
}

fn load_dotenv(path: &str) {
    let Ok(text) = fs::read_to_string(path) else { return };
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            let k = k.trim();
            if env::var_os(k).is_none() {
                // SAFETY: シングルスレッドの起動直後にだけ呼ぶ
                unsafe { env::set_var(k, v.trim()) };
            }
        }
    }
}

fn required(key: &str) -> Result<String, String> {
    match env::var(key) {
        Ok(v) if !v.is_empty() => Ok(v),
        _ => Err(format!("{key} が設定されていない（.env を確認）")),
    }
}

fn opt(key: &str, default: &str) -> String {
    env::var(key).ok().filter(|v| !v.is_empty()).unwrap_or_else(|| default.to_string())
}

fn num(key: &str, default: f64) -> Result<f64, String> {
    match env::var(key).ok().filter(|v| !v.is_empty()) {
        None => Ok(default),
        Some(v) => v.parse().map_err(|_| format!("{key} が数値でない: {v}")),
    }
}
