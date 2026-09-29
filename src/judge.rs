//! 「雨」「雨なし」の 2 状態を、実測の連続回数とクールダウンで切り替える。
//! ぱらつきで通知が連発しないよう、降り始めと止みでしきい値を変えている（ヒステリシス）。

use serde::{Deserialize, Serialize};

use crate::yahoo::{Point, Series};

pub struct Rules {
    /// 降り始め: この強さ [mm/h] 以上が start_count 回連続したら
    pub start_mmh: f64,
    pub start_count: usize,
    /// 止み: この強さ [mm/h] 未満が stop_count 回連続したら
    pub stop_mmh: f64,
    pub stop_count: usize,
    /// 状態が変わってから、逆向きに変えない時間 [分]
    pub cooldown_min: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct State {
    pub raining: bool,
    /// 最後に状態が変わった時刻（JST "YYYYMMDDHHMM"）
    pub changed_at: String,
}

#[derive(Debug, PartialEq)]
pub enum Event {
    /// 降り始めた。stop_at は予測で止んだまま最後まで続く最初の時刻（60 分以内に止まなければ None）
    Started { now_mmh: f64, stop_at: Option<String> },
    /// 止んだ。next_start は予測で次に降り出す時刻（60 分以内に降らなければ None）
    Stopped { next_start: Option<String> },
}

impl Event {
    pub fn kind(&self) -> &'static str {
        match self {
            Event::Started { .. } => "started",
            Event::Stopped { .. } => "stopped",
        }
    }
}

pub fn decide(prev: Option<&State>, s: &Series, r: &Rules) -> (State, Option<Event>) {
    let Some(prev) = prev else {
        // 初回は今の様子を覚えるだけで通知しない（起動しただけで「降ってきました」と言わない）
        let raining = s.observed.last().is_some_and(|p| p.mmh >= r.start_mmh);
        return (State { raining, changed_at: s.now.clone() }, None);
    };

    let cooled = minutes(&s.now) - minutes(&prev.changed_at) >= r.cooldown_min;
    let switched = State { raining: !prev.raining, changed_at: s.now.clone() };

    if !prev.raining && cooled && last_n_all(&s.observed, r.start_count, |v| v >= r.start_mmh) {
        let now_mmh = s.observed.last().map_or(0.0, |p| p.mmh);
        let stop_at = settled_from(&s.forecast, |v| v < r.stop_mmh);
        return (switched, Some(Event::Started { now_mmh, stop_at }));
    }
    if prev.raining && cooled && last_n_all(&s.observed, r.stop_count, |v| v < r.stop_mmh) {
        let next_start = first_time(&s.forecast, |v| v >= r.start_mmh);
        return (switched, Some(Event::Stopped { next_start }));
    }
    (prev.clone(), None)
}

pub fn message(ev: &Event, label: &str) -> String {
    let body = match ev {
        Event::Started { now_mmh, stop_at } => {
            let when = match stop_at {
                Some(t) => format!("**{}ごろ止む見込み**です。", hhmm(t)),
                None => "**1時間以内には止まない見込み**です。".to_string(),
            };
            format!("☔ {label}で雨が降ってきました（{}）。{when}", strength(*now_mmh))
        }
        Event::Stopped { next_start } => {
            let when = match next_start {
                Some(t) => format!("**{}ごろまた降り出す見込み**です。", hhmm(t)),
                None => "**1時間以内は降らない見込み**です。".to_string(),
            };
            format!("🌤 {label}の雨が止みました。{when}")
        }
    };
    format!("{body}\n-# 気象情報: Web Services by Yahoo! JAPAN")
}

/// 気象庁「雨の強さと降り方」の区分をくだけた言い方にしたもの
fn strength(mmh: f64) -> &'static str {
    match mmh {
        v if v < 3.0 => "弱い雨",
        v if v < 10.0 => "やや強めの雨",
        v if v < 20.0 => "強い雨",
        v if v < 30.0 => "土砂降り",
        _ => "激しい雨",
    }
}

fn last_n_all(points: &[Point], n: usize, pred: impl Fn(f64) -> bool) -> bool {
    n > 0 && points.len() >= n && points[points.len() - n..].iter().all(|p| pred(p.mmh))
}

/// 一瞬弱まってまた降る予測で「止む」と言わないよう、そこから予測の終わりまでずっと
/// 条件を満たす最初の時刻を返す
fn settled_from(points: &[Point], pred: impl Fn(f64) -> bool) -> Option<String> {
    let run = points.iter().rev().take_while(|p| pred(p.mmh)).count();
    (run > 0).then(|| points[points.len() - run].time.clone())
}

fn first_time(points: &[Point], pred: impl Fn(f64) -> bool) -> Option<String> {
    points.iter().find(|p| pred(p.mmh)).map(|p| p.time.clone())
}

fn hhmm(t: &str) -> String {
    format!("{}:{}", &t[8..10], &t[10..12])
}

/// "YYYYMMDDHHMM" を通算の分に直す（差を取るだけなので基準日は任意）
fn minutes(t: &str) -> i64 {
    let n = |a: usize, b: usize| t.get(a..b).and_then(|x| x.parse::<i64>().ok()).unwrap_or(0);
    let (y, m, d) = (n(0, 4), n(4, 6), n(6, 8));
    days_from_civil(y, m, d) * 1440 + n(8, 10) * 60 + n(10, 12)
}

/// 暦日から通算日数（Howard Hinnant のアルゴリズム）
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules() -> Rules {
        Rules { start_mmh: 1.0, start_count: 2, stop_mmh: 0.5, stop_count: 3, cooldown_min: 30 }
    }

    /// 22:00 から 5 分刻みの実測と、その後の予測で Series を作る
    fn series(obs: &[f64], fc: &[f64]) -> Series {
        let t = |i: usize| {
            let m = 22 * 60 + i * 5;
            format!("20260929{:02}{:02}", m / 60, m % 60)
        };
        let observed: Vec<Point> = obs.iter().enumerate().map(|(i, &v)| Point { time: t(i), mmh: v }).collect();
        let forecast = fc.iter().enumerate().map(|(i, &v)| Point { time: t(obs.len() + i), mmh: v }).collect();
        Series { now: observed.last().unwrap().time.clone(), observed, forecast }
    }

    fn dry_since(t: &str) -> State {
        State { raining: false, changed_at: t.into() }
    }

    #[test]
    fn first_run_only_remembers() {
        let (st, ev) = decide(None, &series(&[0.0, 2.0], &[2.0]), &rules());
        assert!(st.raining);
        assert_eq!(ev, None);
    }

    #[test]
    fn starts_after_consecutive_rain_with_stop_time() {
        let s = series(&[0.0, 1.2, 1.5], &[1.0, 0.3, 0.0]);
        let (st, ev) = decide(Some(&dry_since("202609291800")), &s, &rules());
        assert!(st.raining);
        assert_eq!(st.changed_at, "202609292210");
        assert_eq!(ev, Some(Event::Started { now_mmh: 1.5, stop_at: Some("202609292220".into()) }));
    }

    #[test]
    fn brief_lull_is_not_a_stop() {
        // 22:20 に一瞬弱まるが 22:25 にまた降る予測。止むのは 22:30 から
        let s = series(&[0.0, 1.2, 1.5], &[0.65, 0.45, 2.13, 0.3, 0.0]);
        let (_, ev) = decide(Some(&dry_since("202609291800")), &s, &rules());
        assert_eq!(ev, Some(Event::Started { now_mmh: 1.5, stop_at: Some("202609292230".into()) }));

        let s = series(&[0.0, 1.2, 1.5], &[0.45, 2.13, 1.45]);
        let (_, ev) = decide(Some(&dry_since("202609291800")), &s, &rules());
        assert_eq!(ev, Some(Event::Started { now_mmh: 1.5, stop_at: None }));
    }

    #[test]
    fn single_blip_does_not_start() {
        let s = series(&[0.0, 0.0, 3.0], &[]);
        let (_, ev) = decide(Some(&dry_since("202609291800")), &s, &rules());
        assert_eq!(ev, None);
    }

    #[test]
    fn cooldown_suppresses_flip() {
        let s = series(&[2.0, 2.0], &[]);
        let (st, ev) = decide(Some(&dry_since("202609292150")), &s, &rules());
        assert!(!st.raining);
        assert_eq!(ev, None);
    }

    #[test]
    fn stops_after_consecutive_dry_without_rain_ahead() {
        let s = series(&[1.0, 0.2, 0.0, 0.0], &[0.0, 0.0]);
        let prev = State { raining: true, changed_at: "202609292000".into() };
        let (st, ev) = decide(Some(&prev), &s, &rules());
        assert!(!st.raining);
        assert_eq!(ev, Some(Event::Stopped { next_start: None }));
    }

    #[test]
    fn stays_raining_while_between_thresholds() {
        let s = series(&[0.7, 0.7, 0.7], &[]);
        let prev = State { raining: true, changed_at: "202609292000".into() };
        assert_eq!(decide(Some(&prev), &s, &rules()).1, None);
    }

    #[test]
    fn cooldown_counts_across_midnight() {
        assert_eq!(minutes("202610010005") - minutes("202609302350"), 15);
    }

    #[test]
    fn messages() {
        let m = message(&Event::Started { now_mmh: 1.5, stop_at: Some("202609292240".into()) }, "自宅まわり");
        assert!(m.starts_with("☔ 自宅まわりで雨が降ってきました（弱い雨）。**22:40ごろ止む見込み**です。"));
        let m = message(&Event::Stopped { next_start: None }, "自宅まわり");
        assert!(m.contains("**1時間以内は降らない見込み**"));
    }
}
