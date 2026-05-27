use crate::errors::AppError;

#[derive(Clone)]
pub struct SupertonicClient {
    client: reqwest::Client,
    base_url: String,
    default_voice: String,
}

impl SupertonicClient {
    pub fn new(base_url: String, default_voice: String) -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url,
            default_voice,
        }
    }

    pub async fn is_available(&self) -> bool {
        self.client
            .get(format!("{}/docs", self.base_url))
            .send()
            .await
            .map(|r| r.status().is_success() || r.status().as_u16() == 404)
            .unwrap_or_else(|_| {
                // try root
                false
            })
            || self
                .client
                .get(&self.base_url)
                .send()
                .await
                .map(|r| r.status().is_success())
                .unwrap_or(false)
    }

    pub async fn synthesize(
        &self,
        text: &str,
        lang: &str,
        voice: Option<&str>,
    ) -> Result<Vec<u8>, AppError> {
        let voice = voice.unwrap_or(&self.default_voice);
        let body = serde_json::json!({
            "model": "supertonic-3",
            "input": text,
            "voice": voice,
            "response_format": "wav",
            "lang": lang,
        });

        let resp = self
            .client
            .post(format!("{}/v1/audio/speech", self.base_url))
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let err = resp.text().await.unwrap_or_default();
            return Err(AppError::Internal(format!(
                "Supertonic TTS error {}: {}",
                status, err
            )));
        }

        let bytes = resp.bytes().await?.to_vec();
        Ok(bytes)
    }
}
