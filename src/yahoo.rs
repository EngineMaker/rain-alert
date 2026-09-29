//! Yahoo! 気象情報 API（YOLP）から、実測と 60 分先までの予測を 5 分刻みで取る。
//! https://developer.yahoo.co.jp/webapi/map/openlocalplatform/v1/weather.html

use serde::{Deserialize, Serialize};

use crate::config::Config;

const ENDPOINT: &str = "https://map.yahooapis.jp/weather/V1/place";

/// 時刻は API の表記のまま（JST の "YYYYMMDDHHMM"）。値は降水強度 [mm/h]。
#[derive(Debug, Clone, Serialize)]
pub struct Point {
    pub time: String,
    pub mmh: f64,
}

#[derive(Debug, Serialize)]
pub struct Series {
    /// 最新の実測の時刻
    pub now: String,
    /// 実測（古い順）。past=2 で過去 2 時間分
    pub observed: Vec<Point>,
    /// 予測（古い順）。now の 5 分後〜60 分後
    pub forecast: Vec<Point>,
}

#[derive(Deserialize)]
struct Response {
    #[serde(rename = "Feature")]
    feature: Vec<Feature>,
}

#[derive(Deserialize)]
struct Feature {
    #[serde(rename = "Property")]
    property: Property,
}

#[derive(Deserialize)]
struct Property {
    #[serde(rename = "WeatherList")]
    weather_list: WeatherList,
}

#[derive(Deserialize)]
struct WeatherList {
    #[serde(rename = "Weather")]
    weather: Vec<Weather>,
}

#[derive(Deserialize)]
struct Weather {
    #[serde(rename = "Type")]
    kind: String,
    #[serde(rename = "Date")]
    date: String,
    #[serde(rename = "Rainfall")]
    rainfall: f64,
}

pub fn fetch(cfg: &Config) -> Result<Series, String> {
    let coordinates = format!("{},{}", cfg.lon, cfg.lat);
    let body = ureq::get(ENDPOINT)
        .query("coordinates", &coordinates)
        .query("output", "json")
        .query("interval", "5")
        .query("past", "2")
        .query("appid", &cfg.appid)
        .call()
        // エラー文に URL（appid 入り）を出さない
        .map_err(|e| format!("Yahoo! 気象情報 API の呼び出しに失敗: {}", redact(&e.to_string(), &cfg.appid)))?
        .body_mut()
        .read_to_string()
        .map_err(|e| format!("Yahoo! の応答を読めない: {e}"))?;
    parse(&body)
}

fn parse(body: &str) -> Result<Series, String> {
    let res: Response =
        serde_json::from_str(body).map_err(|e| format!("Yahoo! の応答が想定外: {e}: {}", truncate(body, 200)))?;
    let list = res
        .feature
        .into_iter()
        .next()
        .ok_or("Yahoo! の応答に Feature がない")?
        .property
        .weather_list
        .weather;

    let mut observed = Vec::new();
    let mut forecast = Vec::new();
    for w in list {
        let p = Point { time: w.date, mmh: w.rainfall };
        match w.kind.as_str() {
            "observation" => observed.push(p),
            "forecast" => forecast.push(p),
            _ => {}
        }
    }
    observed.sort_by(|a, b| a.time.cmp(&b.time));
    forecast.sort_by(|a, b| a.time.cmp(&b.time));
    let now = observed.last().ok_or("Yahoo! の応答に実測がない")?.time.clone();
    Ok(Series { now, observed, forecast })
}

fn redact(s: &str, secret: &str) -> String {
    if secret.is_empty() { s.to_string() } else { s.replace(secret, "***") }
}

fn truncate(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_observation_and_forecast() {
        let body = r#"{"ResultInfo":{"Count":1},"Feature":[{"Property":{"WeatherAreaCode":4410,"WeatherList":{"Weather":[
            {"Type":"observation","Date":"202609292205","Rainfall":0.0},
            {"Type":"observation","Date":"202609292210","Rainfall":1.35},
            {"Type":"forecast","Date":"202609292215","Rainfall":1.65},
            {"Type":"forecast","Date":"202609292220","Rainfall":0.45}]}}}]}"#;
        let s = parse(body).unwrap();
        assert_eq!(s.now, "202609292210");
        assert_eq!(s.observed.len(), 2);
        assert_eq!(s.forecast[1].mmh, 0.45);
    }

    #[test]
    fn error_body_is_reported() {
        let err = parse(r#"{"Error":{"Message":"Your Request was Forbidden"}}"#).unwrap_err();
        assert!(err.contains("Forbidden"));
    }
}
