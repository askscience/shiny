//! The network half of the filter engine.
//!
//! The engine type itself lives in `shiny-filter-core` (no HTTP client, no
//! runtime) so the `peakd` shell can link it. This module re-exports that
//! surface and adds [`load`], which fetches the configured filter lists,
//! compiles them and writes the shared on-disk cache the shell restores from.

pub use shiny_filter_core::engine::*;

/// Load the engine, preferring a valid on-disk cache and falling back to
/// downloading + compiling the configured lists.
///
/// This never fails hard: a browser that cannot fetch EasyList must still
/// browse. On total failure it returns an empty engine and logs why.
pub async fn load(config: FilterConfig) -> AdFilter {
    let cache_dir = config.cache_dir.clone();
    let cache_file = cache_dir.join("engine.dat");
    let meta_file = cache_dir.join("engine.json");

    // 1. Fast path: a cache whose signature still matches the config.
    let signature = signature_of(&config);
    if let Ok(meta_raw) = std::fs::read_to_string(&meta_file) {
        if let Ok(meta) = serde_json::from_str::<CacheMeta>(&meta_raw) {
            if meta.signature == signature {
                if let Ok(bytes) = std::fs::read(&cache_file) {
                    match AdFilter::from_serialized(&bytes, meta.rules) {
                        Ok(filter) => {
                            tracing::info!(
                                "adfilter: loaded {} rules from cache in {:?}",
                                meta.rules,
                                cache_file
                            );
                            return filter.with_config(config);
                        }
                        Err(err) => {
                            tracing::warn!("adfilter: cache unusable ({err}); recompiling");
                        }
                    }
                }
            }
        }
    }

    // 2. Download + compile.
    let mut lists: Vec<String> = Vec::new();
    if !config.offline {
        let client = match wreq::Client::builder()
            .timeout(config.fetch_timeout)
            .user_agent(concat!("shiny-filter/", env!("CARGO_PKG_VERSION")))
            .build()
        {
            Ok(c) => c,
            Err(err) => {
                tracing::warn!("adfilter: HTTP client failed ({err}); continuing unfiltered");
                return AdFilter::empty().with_config(config);
            }
        };
        for url in config.list_urls.clone() {
            match client.get(&url).send().await {
                Ok(resp) if resp.status().is_success() => match resp.text().await {
                    Ok(text) => {
                        tracing::info!("adfilter: fetched {} ({} bytes)", url, text.len());
                        lists.push(text);
                    }
                    Err(err) => tracing::warn!("adfilter: body of {url} failed: {err}"),
                },
                Ok(resp) => tracing::warn!("adfilter: {url} -> HTTP {}", resp.status()),
                Err(err) => tracing::warn!("adfilter: {url} failed: {err}"),
            }
        }
    }
    lists.extend(config.extra_filters.iter().cloned());

    if lists.is_empty() {
        tracing::warn!("adfilter: no filter lists available; running unfiltered");
        return AdFilter::empty().with_config(config);
    }

    let debug = config.debug;
    let filter = tokio::task::spawn_blocking(move || AdFilter::from_lists(lists, debug))
        .await
        .unwrap_or_else(|_| AdFilter::empty());

    // 3. Persist the compiled engine for the next start.
    if let Ok(bytes) = filter.serialize() {
        if std::fs::create_dir_all(&cache_dir).is_ok() {
            let rules = filter.rule_count();
            let meta = CacheMeta {
                signature,
                rules,
                compiled_at: chrono_stamp(),
            };
            if let Err(err) = std::fs::write(&cache_file, &bytes) {
                tracing::warn!("adfilter: could not write cache: {err}");
            } else if let Ok(raw) = serde_json::to_string(&meta) {
                let _ = std::fs::write(&meta_file, raw);
            }
        }
    }

    filter.with_config(config)
}
