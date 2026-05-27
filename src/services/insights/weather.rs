//! Open-Meteo forecast — free, no API key, no registration.
//! https://open-meteo.com/en/docs

use crate::errors::AppError;
use crate::services::insights::types::InsightCard;
use serde::Deserialize;
use uuid::Uuid;

#[derive(Debug, Deserialize)]
struct ForecastResponse {
    daily: Option<DailyForecast>,
}

#[derive(Debug, Deserialize)]
struct DailyForecast {
    time: Vec<String>,
    weather_code: Vec<i32>,
    temperature_2m_max: Vec<f64>,
    temperature_2m_min: Vec<f64>,
}

/// Icon stem under `/icons/insights/` for the primary forecast day.
pub fn weather_icon_stem(code: i32) -> &'static str {
    match code {
        0 => "weather-sun",
        1 | 2 | 3 => "weather-partly",
        45 | 48 => "weather-fog",
        51 | 53 | 55 => "weather-drizzle",
        61 | 63 | 65 | 80 | 81 | 82 => "weather-rain",
        71 | 73 | 75 | 77 => "weather-snow",
        95 | 96 | 99 => "weather-storm",
        _ => "weather-cloud",
    }
}

/// WMO weather interpretation codes (Open-Meteo).
fn weather_label(code: i32) -> &'static str {
    match code {
        0 => "Clear sky",
        1 | 2 | 3 => "Partly cloudy",
        45 | 48 => "Fog",
        51 | 53 | 55 => "Drizzle",
        61 | 63 | 65 => "Rain",
        71 | 73 | 75 => "Snow",
        80 | 81 | 82 => "Rain showers",
        95 | 96 | 99 => "Thunderstorm",
        _ => "Variable conditions",
    }
}

pub async fn fetch_forecast(
    client: &reqwest::Client,
    destination: &str,
    lat: f64,
    lon: f64,
) -> Result<Option<InsightCard>, AppError> {
    let url = format!(
        "https://api.open-meteo.com/v1/forecast?latitude={}&longitude={}\
         &daily=weather_code,temperature_2m_max,temperature_2m_min\
         &timezone=auto&forecast_days=3",
        lat, lon
    );

    let resp = client.get(&url).send().await?;
    if !resp.status().is_success() {
        return Ok(None);
    }

    let data: ForecastResponse = resp.json().await.map_err(|e| {
        AppError::Internal(format!("Open-Meteo parse error: {}", e))
    })?;

    let daily = match data.daily {
        Some(d) if !d.time.is_empty() => d,
        _ => return Ok(None),
    };

    let idx = 0;
    let code = daily.weather_code.get(idx).copied().unwrap_or(0);
    let hi = daily.temperature_2m_max.get(idx).copied().unwrap_or(0.0);
    let lo = daily.temperature_2m_min.get(idx).copied().unwrap_or(0.0);
    let day = daily.time.get(idx).map(|s| s.as_str()).unwrap_or("Today");

    let body = if daily.time.len() > 1 {
        let code2 = daily.weather_code.get(1).copied().unwrap_or(code);
        format!(
            "{} · {}–{:.0}°C. Tomorrow: {}.",
            weather_label(code),
            lo,
            hi,
            weather_label(code2)
        )
    } else {
        format!(
            "{} · highs {:.0}°C, lows {:.0}°C.",
            weather_label(code),
            hi,
            lo
        )
    };

    Ok(Some(InsightCard {
        id: Uuid::new_v4().to_string(),
        kind: "weather".into(),
        title: format!("Weather in {} · {}", destination, day),
        body,
        icon: weather_icon_stem(code).into(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rain_codes_map_to_rain_icon() {
        assert_eq!(weather_icon_stem(65), "weather-rain");
        assert_eq!(weather_icon_stem(0), "weather-sun");
    }
}
