//! Diary generation for the traveler plugin — writes a markdown diary entry
//! for a date from GPS locations + trips, via Ollama. Mirrors core's
//! `DiaryGenerator` (which stays for core REST/cron paths).

use shiny_plugin_sdk::errors::AppError;
use shiny_plugin_sdk::services::PluginCtx;

use crate::models::{DiaryEntry, Location, Trip};
use crate::osm::haversine_km;

pub async fn generate_for_date(
    ctx: &PluginCtx,
    traveler_id: &str,
    date: &str,
) -> Result<DiaryEntry, AppError> {
    let locations = sqlx::query_as::<_, Location>(
        "SELECT * FROM locations WHERE traveler_id = ?1 AND date(timestamp) = ?2 ORDER BY timestamp ASC",
    )
    .bind(traveler_id)
    .bind(date)
    .fetch_all(ctx.pool().await)
    .await?;

    let trips = sqlx::query_as::<_, Trip>(
        "SELECT * FROM trips WHERE traveler_id = ?1 ORDER BY created_at DESC",
    )
    .bind(traveler_id)
    .fetch_all(ctx.pool().await)
    .await?;

    let active_trip = trips.iter().find(|t| {
        t.start_time.as_deref().map(|s| s.starts_with(date)).unwrap_or(false)
    });

    let trip_id = active_trip.map(|t| t.id.clone());
    let trip_name = active_trip.map(|t| t.name.as_str()).unwrap_or("Unknown trip");

    let location_summary = build_location_summary(&locations).await;

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

    let content = ctx
        .ollama()
        .await
        .generate(
            &prompt,
            Some("You are a travel diary writer. Generate concise, factual diary entries in markdown list format."),
            None,
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
    .execute(ctx.pool().await)
    .await?;

    Ok(entry)
}

async fn build_location_summary(locations: &[Location]) -> String {
    if locations.is_empty() {
        return "No GPS data recorded for this date.".into();
    }

    let osm_client = crate::osm::OsmClient::new();
    let mut summary = String::new();
    let mut prev: Option<(f64, f64)> = None;
    let mut total_dist = 0.0;

    for loc in locations {
        if let Some((pl, pn)) = prev {
            total_dist += haversine_km(pl, pn, loc.latitude, loc.longitude);
        }
        prev = Some((loc.latitude, loc.longitude));

        let place = osm_client
            .reverse_geocode(loc.latitude, loc.longitude)
            .await
            .map(|p| p.display_name)
            .unwrap_or_else(|_| format!("{}, {}", loc.latitude, loc.longitude));

        let time = loc.timestamp.as_deref().unwrap_or("unknown");
        let speed = loc
            .speed
            .map(|s| format!("{:.1} km/h", s * 3.6))
            .unwrap_or_else(|| "N/A".into());

        summary.push_str(&format!("- {} (at {}, speed: {})\n", place, time, speed));
    }

    summary.push_str(&format!("\nTotal distance traveled: {:.2} km", total_dist));
    summary
}
