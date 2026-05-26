use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::net::TcpStream;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::models::Location;

#[derive(Debug, Clone)]
pub struct GpsPosition {
    pub latitude: f64,
    pub longitude: f64,
    pub altitude: Option<f64>,
    pub speed: Option<f64>,
    pub heading: Option<f64>,
    pub timestamp: String,
}

#[derive(Clone)]
pub struct GpsdService {
    host: String,
    port: u16,
    current: Arc<Mutex<GpsPosition>>,
    connected: Arc<Mutex<bool>>,
}

impl GpsdService {
    pub fn new(host: String, port: u16) -> Self {
        let now = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();
        Self {
            host,
            port,
            current: Arc::new(Mutex::new(GpsPosition {
                latitude: 0.0,
                longitude: 0.0,
                altitude: None,
                speed: None,
                heading: None,
                timestamp: now,
            })),
            connected: Arc::new(Mutex::new(false)),
        }
    }

    pub async fn start(&self) {
        let host = self.host.clone();
        let port = self.port;
        let current = self.current.clone();
        let connected = self.connected.clone();

        tokio::spawn(async move {
            let addr = format!("{}:{}", host, port);

            match TcpStream::connect(&addr).await {
                Ok(stream) => {
                    tracing::info!("Connected to GPSD at {}", addr);
                    *connected.lock().await = true;

                    let (reader, mut writer) = stream.into_split();

                    let watch_cmd = "?WATCH={\"enable\":true,\"json\":true}\n";
                    if let Err(e) = writer.write_all(watch_cmd.as_bytes()).await {
                        tracing::warn!("Failed to send GPSD watch command: {}", e);
                        *connected.lock().await = false;
                        return;
                    }

                    let mut buf_reader = BufReader::new(reader);
                    let mut line = String::new();

                    loop {
                        line.clear();
                        match buf_reader.read_line(&mut line).await {
                            Ok(0) => {
                                tracing::warn!("GPSD connection closed");
                                *connected.lock().await = false;
                                break;
                            }
                            Ok(_) => {
                                if let Some(pos) = parse_tpv(&line) {
                                    *current.lock().await = pos;
                                }
                            }
                            Err(e) => {
                                tracing::warn!("GPSD read error: {}", e);
                                *connected.lock().await = false;
                                break;
                            }
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        "Cannot connect to GPSD at {} ({}). Using mock data.",
                        addr, e
                    );
                    *connected.lock().await = false;
                }
            }

            Self::mock_gps_loop(current, connected).await;
        });
    }

    async fn mock_gps_loop(
        current: Arc<Mutex<GpsPosition>>,
        _connected: Arc<Mutex<bool>>,
    ) {
        let mut lat = 48.8566;
        let mut lon = 2.3522;

        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;

            lat += (rand::random::<f64>() - 0.5) * 0.001;
            lon += (rand::random::<f64>() - 0.5) * 0.001;

            let now = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();
            let pos = GpsPosition {
                latitude: lat,
                longitude: lon,
                altitude: Some(35.0 + rand::random::<f64>() * 5.0),
                speed: Some(rand::random::<f64>() * 30.0),
                heading: Some(rand::random::<f64>() * 360.0),
                timestamp: now,
            };

            *current.lock().await = pos;
        }
    }

    pub async fn get_current_position(&self) -> GpsPosition {
        self.current.lock().await.clone()
    }

    pub async fn is_connected(&self) -> bool {
        *self.connected.lock().await
    }

    pub fn to_location(&self, pos: &GpsPosition, traveler_id: &str, trip_id: Option<&str>) -> Location {
        Location::new(
            traveler_id.to_string(),
            trip_id.map(|s| s.to_string()),
            pos.latitude,
            pos.longitude,
            pos.altitude,
            pos.speed,
            pos.heading,
            "gps".into(),
        )
    }
}

fn parse_tpv(line: &str) -> Option<GpsPosition> {
    if !line.contains("\"class\":\"TPV\"") {
        return None;
    }

    let v: serde_json::Value = serde_json::from_str(line).ok()?;

    let lat = v.get("lat")?.as_f64()?;
    let lon = v.get("lon")?.as_f64()?;

    let now = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();

    Some(GpsPosition {
        latitude: lat,
        longitude: lon,
        altitude: v.get("alt").and_then(|v| v.as_f64()),
        speed: v.get("speed").and_then(|v| v.as_f64()),
        heading: v.get("track").and_then(|v| v.as_f64()),
        timestamp: v
            .get("time")
            .and_then(|v| v.as_str())
            .unwrap_or(&now)
            .to_string(),
    })
}
