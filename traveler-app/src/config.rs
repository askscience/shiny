use std::env;

#[derive(Clone, Debug)]
pub struct Config {
    pub server_host: String,
    pub server_port: u16,
    pub database_url: String,
    pub ollama_url: String,
    pub ollama_model: String,
    pub gpsd_host: String,
    pub gpsd_port: u16,
    pub diary_auto_generate: bool,
    pub diary_generate_time: String,
    pub log_level: String,
}

impl Config {
    pub fn from_env() -> Self {
        Self {
            server_host: env::var("SERVER_HOST").unwrap_or_else(|_| "0.0.0.0".into()),
            server_port: env::var("SERVER_PORT")
                .unwrap_or_else(|_| "8080".into())
                .parse()
                .unwrap_or(8080),
            database_url: env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite:data/traveler.db".into()),
            ollama_url: env::var("OLLAMA_URL").unwrap_or_else(|_| "http://ollama.local:11434".into()),
            ollama_model: env::var("OLLAMA_MODEL").unwrap_or_else(|_| "gemma4:31b-cloud".into()),
            gpsd_host: env::var("GPSD_HOST").unwrap_or_else(|_| "127.0.0.1".into()),
            gpsd_port: env::var("GPSD_PORT")
                .unwrap_or_else(|_| "2947".into())
                .parse()
                .unwrap_or(2947),
            diary_auto_generate: env::var("DIARY_AUTO_GENERATE")
                .unwrap_or_else(|_| "true".into())
                .parse()
                .unwrap_or(true),
            diary_generate_time: env::var("DIARY_GENERATE_TIME").unwrap_or_else(|_| "21:00".into()),
            log_level: env::var("LOG_LEVEL").unwrap_or_else(|_| "info".into()),
        }
    }
}
