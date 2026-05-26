use sqlx::SqlitePool;
use tokio::fs;

use crate::errors::AppError;
use crate::models::{DiaryEntry, Location, Trip};
use crate::services::ollama::OllamaClient;
use crate::services::osm::OsmService;

pub struct DiaryGenerator {
    pool: SqlitePool,
    ollama: OllamaClient,
    osm: OsmService,
}

impl DiaryGenerator {
    pub fn new(pool: SqlitePool, ollama: OllamaClient, osm: OsmService) -> Self {
        Self {
            pool,
            ollama,
            osm,
        }
    }

    pub async fn generate_for_date(
        &self,
        traveler_id: &str,
        date: &str,
    ) -> Result<DiaryEntry, AppError> {
        let locations = sqlx::query_as::<_, Location>(
            "SELECT * FROM locations WHERE traveler_id = ?1 AND date(timestamp) = ?2 ORDER BY timestamp ASC",
        )
        .bind(traveler_id)
        .bind(date)
        .fetch_all(&self.pool)
        .await?;

        let trips = sqlx::query_as::<_, Trip>(
            "SELECT * FROM trips WHERE traveler_id = ?1 ORDER BY created_at DESC",
        )
        .bind(traveler_id)
        .fetch_all(&self.pool)
        .await?;

        let active_trip = trips.iter().find(|t| {
            t.start_time.as_deref().map(|s| s.starts_with(date)).unwrap_or(false)
        });

        let trip_id = active_trip.map(|t| t.id.clone());
        let trip_name = active_trip.map(|t| t.name.as_str()).unwrap_or("Unknown trip");

        let location_summary = self.build_location_summary(&locations).await;

        let prompt = format!(
            "Generate a travel diary entry for {date}. \
             The format must be a markdown list. Each list item should be: \
             - **Place name** (lat, lon): description\n\n\
             Location data for this date:\n{locations}\n\n\
             Trip name: {trip_name}\n\n\
             Important rules:\n\
             1. Write in first person\n\
             2. Each line must be a list item starting with '- '\n\
             3. Include coordinates where available\n\
             4. Estimate activities based on time spent at locations\n\
             5. Add a total distance estimate at the end\n\
             6. End with *Total distance: X km. Weather: N/A.*",
            date = date,
            locations = location_summary,
            trip_name = trip_name,
        );

        let content = self
            .ollama
            .generate(
                &prompt,
                Some("You are a travel diary writer. Generate concise, factual diary entries in markdown list format."),
            )
            .await?;

        let title = format!("Travel Diary - {}", date);

        let entry = DiaryEntry::new(
            traveler_id.to_string(),
            trip_id,
            date.to_string(),
            content.clone(),
            true,
        );

        sqlx::query(
            "INSERT INTO diary_entries (id, traveler_id, trip_id, date, title, content_markdown, summary, auto_generated, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, datetime('now'))",
        )
        .bind(&entry.id)
        .bind(&entry.traveler_id)
        .bind(&entry.trip_id)
        .bind(&entry.date)
        .bind(&title)
        .bind(&entry.content_markdown)
        .bind(&content[..content.len().min(200)])
        .bind(entry.auto_generated)
        .execute(&self.pool)
        .await?;

        let diary_dir = "diaries";
        fs::create_dir_all(diary_dir).await.unwrap_or_default();
        let file_path = format!("{}/{}.md", diary_dir, date);
        let file_content = format!("# {}\n\n{}", title, content);
        fs::write(&file_path, &file_content).await.unwrap_or_else(|e| {
            tracing::warn!("Failed to write diary file {}: {}", file_path, e);
        });

        Ok(entry)
    }

    async fn build_location_summary(&self, locations: &[Location]) -> String {
        if locations.is_empty() {
            return "No GPS data recorded for this date.".into();
        }

        let mut summary = String::new();
        let mut prev_lat: Option<f64> = None;
        let mut prev_lon: Option<f64> = None;
        let mut total_dist = 0.0;

        for loc in locations {
            if let (Some(pl), Some(pn)) = (prev_lat, prev_lon) {
                let dist = haversine_distance(pl, pn, loc.latitude, loc.longitude);
                total_dist += dist;
            }
            prev_lat = Some(loc.latitude);
            prev_lon = Some(loc.longitude);

            let place = self
                .osm
                .reverse_geocode(loc.latitude, loc.longitude)
                .await
                .map(|p| p.display_name)
                .unwrap_or_else(|_| format!("{}, {}", loc.latitude, loc.longitude));

            let time = loc.timestamp.as_deref().unwrap_or("unknown");
            let speed = loc
                .speed
                .map(|s| format!("{:.1} km/h", s * 3.6))
                .unwrap_or_else(|| "N/A".into());

            summary.push_str(&format!(
                "- {} (at {}, speed: {})\n",
                place, time, speed
            ));
        }

        summary.push_str(&format!("\nTotal distance traveled: {:.2} km", total_dist));
        summary
    }

    pub async fn auto_generate_daily(&self) {
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();

        let travelers = sqlx::query_as::<_, crate::models::Traveler>(
            "SELECT * FROM travelers WHERE auth_token IS NOT NULL",
        )
        .fetch_all(&self.pool)
        .await;

        match travelers {
            Ok(travelers) => {
                for traveler in travelers {
                    if let Err(e) = self
                        .generate_for_date(&traveler.id, &today)
                        .await
                    {
                        tracing::warn!(
                            "Failed to auto-generate diary for traveler {}: {:?}",
                            traveler.id,
                            e
                        );
                    } else {
                        tracing::info!(
                            "Auto-generated diary for traveler {} on {}",
                            traveler.id,
                            today
                        );
                    }
                }
            }
            Err(e) => {
                tracing::warn!("Failed to fetch travelers for auto-diary: {}", e);
            }
        }
    }
}

fn haversine_distance(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let r = 6371.0;
    let d_lat = (lat2 - lat1).to_radians();
    let d_lon = (lon2 - lon1).to_radians();
    let a = (d_lat / 2.0).sin().powi(2)
        + lat1.to_radians().cos() * lat2.to_radians().cos() * (d_lon / 2.0).sin().powi(2);
    let c = 2.0 * a.sqrt().asin();
    r * c
}
