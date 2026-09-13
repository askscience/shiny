//! Conversation memory: groups chat_messages into resumable threads so the
//! agent keeps the context of each conversation instead of one flat history.
//! Only text is stored — never audio.

use sqlx::SqlitePool;

use crate::errors::AppError;
use crate::services::ai::AiClient;

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
/// `rowid` breaks ties: a turn's user+assistant rows share a
/// second-resolution `timestamp`, so without it their order can swap.
pub async fn recent_history(
    pool: &SqlitePool,
    conversation_id: &str,
    limit: i64,
) -> Result<Vec<(String, String)>, AppError> {
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT role, content FROM (\
             SELECT role, content, timestamp, rowid FROM chat_messages \
             WHERE conversation_id = ?1 ORDER BY timestamp DESC, rowid DESC LIMIT ?2\
         ) ORDER BY timestamp ASC, rowid ASC",
    )
    .bind(conversation_id)
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(AppError::Database)?;
    Ok(rows)
}

/// Invisible note written to a conversation when the user stops the assistant.
///
/// It is stored as a `system`-role row: the agent reads every role back into
/// its history prompt, while `/api/chat/conversations/:id` filters `system`
/// rows out, so the user never sees it in the chat.
pub const INTERRUPTED_NOTE: &str = "The user stopped this reply before it finished. \
Treat the assistant's previous message as incomplete and possibly unread or unheard — \
the user may restate, correct or replace their request next. Do not apologise for the \
interruption or mention this note; just continue naturally.";

/// Persist the user message and the assistant reply, touch the conversation,
/// and set its title from the first user message when still untitled.
pub async fn save_turn(
    pool: &SqlitePool,
    ai: &AiClient,
    traveler_id: &str,
    conversation_id: &str,
    user_message: &str,
    assistant_reply: &str,
) -> Result<(), AppError> {
    save_turn_with_note(
        pool,
        ai,
        traveler_id,
        conversation_id,
        user_message,
        assistant_reply,
        None,
    )
    .await
}

/// Same as [`save_turn`], plus an optional invisible `system` note describing
/// how the turn ended (e.g. the user stopped it).
pub async fn save_turn_with_note(
    pool: &SqlitePool,
    ai: &AiClient,
    traveler_id: &str,
    conversation_id: &str,
    user_message: &str,
    assistant_reply: &str,
    note: Option<&str>,
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

    // A stopped turn may have produced nothing at all; an empty assistant
    // bubble in the history would be noise, so only the note is written.
    if !assistant_reply.trim().is_empty() {
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
    }

    if let Some(note) = note {
        insert_note(pool, traveler_id, conversation_id, note).await?;
    }

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
        let title = generate_title(ai, user_message).await;
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

/// Record an invisible note on a conversation whose turn already finished.
///
/// This is the late-stop path: the answer was generated and saved, and the user
/// stopped it while reading or listening. The model still deserves to know the
/// reply was not taken in full.
pub async fn append_note(
    pool: &SqlitePool,
    traveler_id: &str,
    conversation_id: &str,
    note: &str,
) -> Result<(), AppError> {
    // Only the conversation's owner may annotate it.
    let owned: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM chat_conversations WHERE id = ?1 AND traveler_id = ?2",
    )
    .bind(conversation_id)
    .bind(traveler_id)
    .fetch_one(pool)
    .await
    .map_err(AppError::Database)?;
    if owned == 0 {
        return Ok(());
    }

    insert_note(pool, traveler_id, conversation_id, note).await?;
    sqlx::query("UPDATE chat_conversations SET updated_at = datetime('now') WHERE id = ?1")
        .bind(conversation_id)
        .execute(pool)
        .await
        .map_err(AppError::Database)?;
    Ok(())
}

async fn insert_note(
    pool: &SqlitePool,
    traveler_id: &str,
    conversation_id: &str,
    note: &str,
) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO chat_messages (id, traveler_id, conversation_id, role, content, timestamp) \
         VALUES (?1, ?2, ?3, 'system', ?4, datetime('now'))",
    )
    .bind(uuid::Uuid::new_v4().to_string())
    .bind(traveler_id)
    .bind(conversation_id)
    .bind(note)
    .execute(pool)
    .await
    .map_err(AppError::Database)?;
    Ok(())
}

/// Ask the LLM for a short (three-word) title for a new conversation; falls
/// back to a truncated copy of the message when the AI is unavailable.
async fn generate_title(ai: &AiClient, message: &str) -> String {
    let system = "You title chat conversations. Reply with exactly three words, lowercase, no punctuation, no quotes, no markdown.".to_string();
    match ai
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
