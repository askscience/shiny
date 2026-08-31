//! Conversation memory: groups chat_messages into resumable threads so the
//! agent keeps the context of each conversation instead of one flat history.
//! Only text is stored — never audio.

use sqlx::SqlitePool;

use crate::errors::AppError;
use crate::services::ollama::OllamaClient;

/// Resolve (or create) the conversation a message belongs to. Returns its id.
pub async fn resolve_conversation(
    pool: &SqlitePool,
    traveler_id: &str,
    conversation_id: Option<&str>,
) -> Result<String, AppError> {
    if let Some(id) = conversation_id {
        let exists: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM chat_conversations WHERE id = ?1 AND traveler_id = ?2",
        )
        .bind(id)
        .bind(traveler_id)
        .fetch_one(pool)
        .await
        .map_err(AppError::Database)?;
        if exists > 0 {
            return Ok(id.to_string());
        }
    }

    let id = uuid::Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO chat_conversations (id, traveler_id, title, created_at, updated_at) \
         VALUES (?1, ?2, 'New chat', datetime('now'), datetime('now'))",
    )
    .bind(&id)
    .bind(traveler_id)
    .execute(pool)
    .await
    .map_err(AppError::Database)?;
    Ok(id)
}

/// Load the most recent messages of a conversation, oldest first.
pub async fn recent_history(
    pool: &SqlitePool,
    conversation_id: &str,
    limit: i64,
) -> Result<Vec<(String, String)>, AppError> {
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT role, content FROM (\
             SELECT role, content, timestamp FROM chat_messages \
             WHERE conversation_id = ?1 ORDER BY timestamp DESC LIMIT ?2\
         ) ORDER BY timestamp ASC",
    )
    .bind(conversation_id)
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(AppError::Database)?;
    Ok(rows)
}

/// Persist the user message and the assistant reply, touch the conversation,
/// and set its title from the first user message when still untitled.
pub async fn save_turn(
    pool: &SqlitePool,
    ollama: &OllamaClient,
    traveler_id: &str,
    conversation_id: &str,
    user_message: &str,
    assistant_reply: &str,
) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO chat_messages (id, traveler_id, conversation_id, role, content, timestamp) \
         VALUES (?1, ?2, ?3, 'user', ?4, datetime('now'))",
    )
    .bind(uuid::Uuid::new_v4().to_string())
    .bind(traveler_id)
    .bind(conversation_id)
    .bind(user_message)
    .execute(pool)
    .await
    .map_err(AppError::Database)?;

    sqlx::query(
        "INSERT INTO chat_messages (id, traveler_id, conversation_id, role, content, timestamp) \
         VALUES (?1, ?2, ?3, 'assistant', ?4, datetime('now'))",
    )
    .bind(uuid::Uuid::new_v4().to_string())
    .bind(traveler_id)
    .bind(conversation_id)
    .bind(assistant_reply)
    .execute(pool)
    .await
    .map_err(AppError::Database)?;

    // Title the conversation on its FIRST turn only; later turns just bump
    // updated_at.
    let current_title: Option<String> = sqlx::query_scalar(
        "SELECT title FROM chat_conversations WHERE id = ?1",
    )
    .bind(conversation_id)
    .fetch_optional(pool)
    .await?;

    let is_untitled = matches!(current_title.as_deref(), None | Some("") | Some("New chat"));
    if is_untitled {
        let title = generate_title(ollama, user_message).await;
        sqlx::query(
            "UPDATE chat_conversations SET title = ?1, updated_at = datetime('now') WHERE id = ?2",
        )
        .bind(&title)
        .bind(conversation_id)
        .execute(pool)
        .await
        .map_err(AppError::Database)?;
    } else {
        sqlx::query("UPDATE chat_conversations SET updated_at = datetime('now') WHERE id = ?1")
            .bind(conversation_id)
            .execute(pool)
            .await
            .map_err(AppError::Database)?;
    }

    Ok(())
}

/// Ask the LLM for a short (three-word) title for a new conversation; falls
/// back to a truncated copy of the message when the AI is unavailable.
async fn generate_title(ollama: &OllamaClient, message: &str) -> String {
    let system = "You title chat conversations. Reply with exactly three words, lowercase, no punctuation, no quotes, no markdown.".to_string();
    match ollama
        .chat(
            vec![
                ("system".to_string(), system),
                ("user".to_string(), message.to_string()),
            ],
            None,
        )
        .await
    {
        Ok(title) => {
            let words: Vec<String> = title
                .split_whitespace()
                .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase())
                .filter(|w| !w.is_empty())
                .take(3)
                .collect();
            if words.is_empty() {
                title_from(message)
            } else {
                words.join(" ")
            }
        }
        Err(_) => title_from(message),
    }
}

fn title_from(message: &str) -> String {
    let cleaned = message
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let mut chars = cleaned.chars();
    let mut title: String = chars.by_ref().take(48).collect();
    if chars.next().is_some() {
        title.push('…');
    }
    if title.is_empty() {
        "New chat".to_string()
    } else {
        title
    }
}
