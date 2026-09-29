//! 気象庁 降水短時間予報（15 時間先まで・1 時間ごと）から、1 時間より先の「何時ごろ」の目安を出す。
//! 数値の API は無いので、地図タイル（PNG）の自宅の地点の色を凡例の色に当てはめて雨量の区分を読む。
//! 公式に案内された API ではないので、失敗したら目安なしで続ける前提（呼び出し側で握りつぶす）。

use std::collections::BTreeMap;
use std::f64::consts::PI;
use std::fs;
use std::path::Path;

use serde::Deserialize;

const BASE: &str = "https://www.jma.go.jp/bosai/jmatile/data/rasrf";
const ZOOM: u32 = 10;

/// 1 時間雨量の区分。凡例の色（気象庁の降水短時間予報）と対応
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    None,
    Under1,
    From1,
    From5,
    From10,
    From20,
    From30,
    From50,
    From80,
}

const LEGEND: [((u8, u8, u8), Level); 8] = [
    ((242, 242, 255), Level::Under1),
    ((160, 210, 255), Level::From1),
    ((33, 140, 255), Level::From5),
    ((0, 65, 255), Level::From10),
    ((250, 245, 0), Level::From20),
    ((255, 153, 0), Level::From30),
    ((255, 40, 0), Level::From50),
    ((180, 0, 104), Level::From80),
];

/// 1 時間分の予報。end は時間帯の終わり（UTC "YYYYMMDDHHMMSS"）
#[derive(Debug, Clone)]
pub struct Hour {
    pub end: String,
    pub level: Level,
}

/// 降り出しの目安
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum StartHint {
    /// この時（JST、0〜23）ごろから降りそう
    At(u32),
    /// 15 時間以内は降らなそう
    NotWithin,
}

#[derive(Deserialize)]
struct Target {
    basetime: String,
    validtime: String,
    member: String,
    elements: Vec<String>,
}

/// 止む目安: 1mm 未満が 3 時間（その時間帯＋後の 2 時間）続く最初の時間帯の終わりの時（JST）
pub fn stop_hour(hours: &[Hour]) -> Option<u32> {
    let dry = |h: &Hour| h.level <= Level::Under1;
    (0..hours.len())
        .find(|&i| hours[i..(i + 3).min(hours.len())].iter().all(dry))
        .map(|i| jst_hour_rounded(&hours[i].end, 0))
}

/// 降り出す目安: 1mm 以上になる最初の時間帯の始まりの時（JST）
pub fn start_hint(hours: &[Hour]) -> StartHint {
    match hours.iter().find(|h| h.level >= Level::From1) {
        Some(h) => StartHint::At(jst_hour_rounded(&h.end, -1)),
        None => StartHint::NotWithin,
    }
}

/// 自宅の地点の、この先 15 時間の 1 時間ごとの予報。タイルの読み取り結果は data_dir にキャッシュする
pub fn fetch(lat: f64, lon: f64, data_dir: &str) -> Result<Vec<Hour>, String> {
    let targets: Vec<Target> = serde_json::from_str(&get_text(&format!("{BASE}/targetTimes.json"))?)
        .map_err(|e| format!("気象庁 targetTimes.json が想定外: {e}"))?;

    // 予報時刻ごとに、いちばん新しい発表を使う（1〜6 時間先は 10 分ごと、7〜15 時間先は毎正時の発表）
    let mut latest: BTreeMap<String, (String, String)> = BTreeMap::new();
    for t in targets {
        if !t.elements.iter().any(|e| e == "rasrf") || t.validtime <= t.basetime {
            continue;
        }
        let cand = (t.basetime, t.member);
        match latest.get(&t.validtime) {
            Some(cur) if cur.0 >= cand.0 => {}
            _ => {
                latest.insert(t.validtime, cand);
            }
        }
    }

    let cache_path = Path::new(data_dir).join("jma-cache.json");
    let old: BTreeMap<String, u8> =
        fs::read_to_string(&cache_path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    let mut cache = BTreeMap::new();

    let (tx, ty, px, py) = tile_of(lat, lon, ZOOM);
    let mut hours = Vec::new();
    for (vt, (bt, member)) in latest {
        let key = format!("{bt}/{member}/{vt}");
        let level = match old.get(&key) {
            Some(&n) => from_u8(n),
            None => {
                let url = format!("{BASE}/{bt}/{member}/{vt}/surf/rasrf/{ZOOM}/{tx}/{ty}.png");
                read_level(&get_bytes(&url)?, px, py)?
            }
        };
        cache.insert(key, level as u8);
        hours.push(Hour { end: vt, level });
    }
    // 今回の発表の分だけ残す（古いものは捨てる）
    if let Ok(text) = serde_json::to_string(&cache) {
        let _ = fs::write(&cache_path, text);
    }
    Ok(hours)
}

fn read_level(png_bytes: &[u8], px: u32, py: u32) -> Result<Level, String> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(png_bytes));
    // パレット形式でも RGB(A) に展開して読む
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(|e| format!("気象庁のタイルを読めない: {e}"))?;
    let mut buf = vec![0; reader.output_buffer_size().ok_or("気象庁のタイルが大きすぎる")?];
    let info = reader.next_frame(&mut buf).map_err(|e| format!("気象庁のタイルを読めない: {e}"))?;
    let ch = info.color_type.samples();
    let i = (py as usize * info.width as usize + px as usize) * ch;
    let p = buf.get(i..i + ch).ok_or("気象庁のタイルの大きさが想定外")?;
    let (rgb, alpha) = match ch {
        4 => ((p[0], p[1], p[2]), p[3]),
        3 => ((p[0], p[1], p[2]), 255),
        2 => ((p[0], p[0], p[0]), p[1]),
        _ => ((p[0], p[0], p[0]), 255),
    };
    classify(rgb, alpha)
}

fn classify(rgb: (u8, u8, u8), alpha: u8) -> Result<Level, String> {
    if alpha == 0 {
        return Ok(Level::None);
    }
    let dist = |c: (u8, u8, u8)| {
        let d = |a: u8, b: u8| (a as i32 - b as i32).pow(2);
        d(c.0, rgb.0) + d(c.1, rgb.1) + d(c.2, rgb.2)
    };
    let (color, level) = LEGEND.iter().min_by_key(|(c, _)| dist(*c)).expect("凡例は空でない");
    // 凡例に無い色（配信の仕様変更など）は読み違えないよう失敗にする
    if dist(*color) > 300 {
        return Err(format!("気象庁のタイルに凡例に無い色 {rgb:?}"));
    }
    Ok(*level)
}

fn from_u8(n: u8) -> Level {
    [
        Level::None,
        Level::Under1,
        Level::From1,
        Level::From5,
        Level::From10,
        Level::From20,
        Level::From30,
        Level::From50,
        Level::From80,
    ]
    .get(n as usize)
    .copied()
    .unwrap_or(Level::None)
}

/// 緯度経度 → (タイル x, タイル y, タイル内の x, タイル内の y)
fn tile_of(lat: f64, lon: f64, z: u32) -> (u32, u32, u32, u32) {
    let n = 2f64.powi(z as i32);
    let x = (lon + 180.0) / 360.0 * n;
    let lat_r = lat.to_radians();
    let y = (1.0 - (lat_r.tan() + 1.0 / lat_r.cos()).ln() / PI) / 2.0 * n;
    (x as u32, y as u32, (x.fract() * 256.0) as u32, (y.fract() * 256.0) as u32)
}

/// UTC "YYYYMMDDHHMMSS" を JST の時（0〜23）に。offset 時間ずらしてから 30 分で四捨五入
fn jst_hour_rounded(t: &str, offset: i32) -> u32 {
    let n = |a: usize, b: usize| t.get(a..b).and_then(|x| x.parse::<i32>().ok()).unwrap_or(0);
    let h = n(8, 10) + 9 + offset + if n(10, 12) >= 30 { 1 } else { 0 };
    h.rem_euclid(24) as u32
}

fn get_text(url: &str) -> Result<String, String> {
    ureq::get(url)
        .call()
        .map_err(|e| format!("気象庁 {url} の取得に失敗: {e}"))?
        .body_mut()
        .read_to_string()
        .map_err(|e| format!("気象庁 {url} を読めない: {e}"))
}

fn get_bytes(url: &str) -> Result<Vec<u8>, String> {
    ureq::get(url)
        .call()
        .map_err(|e| format!("気象庁 {url} の取得に失敗: {e}"))?
        .body_mut()
        .read_to_vec()
        .map_err(|e| format!("気象庁 {url} を読めない: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 01:20 JST（16:20 UTC）に終わる時間帯から 1 時間ごと（今夜の実際の並び）
    fn hours(levels: &[Level]) -> Vec<Hour> {
        levels
            .iter()
            .enumerate()
            .map(|(i, &level)| Hour { end: format!("20260929{:02}2000", 16 + i), level })
            .collect()
    }

    use Level::*;

    #[test]
    fn stop_needs_three_dry_hours() {
        // 1〜5mm が 4 時間、1mm 未満（04:20〜05:20）、以降なし → 5 時ごろまでに止む（今夜の予報）
        let h = hours(&[From1, From1, From1, From1, Under1, None, None, None]);
        assert_eq!(stop_hour(&h), Some(5));
        // 一度弱まってまた降るのは止みにしない（02:20 までの 1mm 未満は飛ばす）
        let h = hours(&[From1, Under1, From5, From1, None, None, None]);
        assert_eq!(stop_hour(&h), Some(5));
        assert_eq!(stop_hour(&hours(&[From1, From1, From1])), Option::None);
    }

    #[test]
    fn start_hint_uses_window_start() {
        // 07:20〜08:20 の時間帯から降る → 7 時ごろから
        let h = hours(&[None, None, None, None, None, None, None, From1]);
        assert_eq!(start_hint(&h), StartHint::At(7));
        assert_eq!(start_hint(&hours(&[None, Under1, None])), StartHint::NotWithin);
    }

    #[test]
    fn classifies_legend_colors() {
        assert_eq!(classify((160, 210, 255), 255), Ok(From1));
        assert_eq!(classify((0, 0, 0), 0), Ok(None));
        assert!(classify((10, 200, 10), 255).is_err());
    }

    #[test]
    fn tile_of_home_area() {
        // 新宿付近（z=10）
        let (x, y, ..) = tile_of(35.6846, 139.7163, 10);
        assert_eq!((x, y), (909, 403));
    }
}
