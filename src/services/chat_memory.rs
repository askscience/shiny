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
             WHERE conversation_id = ?1 AND role IN ('user', 'assistant', 'system') \
             ORDER BY timestamp DESC, rowid DESC LIMIT ?2\
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
/// Persist one turn in a single transaction. On the first turn the conversation
/// gets a local title immediately (no second model call before `Done`); the
/// caller may upgrade it in the background with [`upgrade_title`].
///
/// Returns the local title when this call set one, so the caller knows whether
/// a background upgrade is useful.
pub async fn save_turn_with_note(
    pool: &SqlitePool,
    traveler_id: &str,
    conversation_id: &str,
    user_message: &str,
    assistant_reply: &str,
    note: Option<&str>,
    assistant_thinking: Option<&str>,
    events: &[(String, String)],
) -> Result<Option<String>, AppError> {
    let mut tx = pool.begin().await.map_err(AppError::Database)?;

    sqlx::query(
        "INSERT INTO chat_messages (id, traveler_id, conversation_id, role, content, timestamp) \
         VALUES (?1, ?2, ?3, 'user', ?4, datetime('now'))",
    )
    .bind(uuid::Uuid::new_v4().to_string())
    .bind(traveler_id)
    .bind(conversation_id)
    .bind(user_message)
    .execute(&mut *tx)
    .await
    .map_err(AppError::Database)?;

    // Terminal command records belong between the user's message and the
    // answer — the same place the live approval cards appear.
    for (role, content) in events.iter().filter(|(role, _)| role.as_str() != "media") {
        sqlx::query(
            "INSERT INTO chat_messages (id, traveler_id, conversation_id, role, content, timestamp) \
             VALUES (?1, ?2, ?3, ?4, ?5, datetime('now'))",
        )
        .bind(uuid::Uuid::new_v4().to_string())
        .bind(traveler_id)
        .bind(conversation_id)
        .bind(role.as_str())
        .bind(content.as_str())
        .execute(&mut *tx)
        .await
        .map_err(AppError::Database)?;
    }

    // A stopped turn may have produced nothing at all; an empty assistant
    // bubble in the history would be noise, so only the note is written.
    if !assistant_reply.trim().is_empty() {
        sqlx::query(
            "INSERT INTO chat_messages (id, traveler_id, conversation_id, role, content, thinking, timestamp) \
             VALUES (?1, ?2, ?3, 'assistant', ?4, ?5, datetime('now'))",
        )
        .bind(uuid::Uuid::new_v4().to_string())
        .bind(traveler_id)
        .bind(conversation_id)
        .bind(assistant_reply)
        .bind(assistant_thinking)
        .execute(&mut *tx)
        .await
        .map_err(AppError::Database)?;
    }

    // Generated images/tracks are their own chat elements below the answer,
    // exactly where the live turn puts them.
    for (role, content) in events.iter().filter(|(role, _)| role.as_str() == "media") {
        sqlx::query(
            "INSERT INTO chat_messages (id, traveler_id, conversation_id, role, content, timestamp) \
             VALUES (?1, ?2, ?3, ?4, ?5, datetime('now'))",
        )
        .bind(uuid::Uuid::new_v4().to_string())
        .bind(traveler_id)
        .bind(conversation_id)
        .bind(role.as_str())
        .bind(content.as_str())
        .execute(&mut *tx)
        .await
        .map_err(AppError::Database)?;
    }

    if let Some(note) = note {
        sqlx::query(
            "INSERT INTO chat_messages (id, traveler_id, conversation_id, role, content, timestamp) \
             VALUES (?1, ?2, ?3, 'system', ?4, datetime('now'))",
        )
        .bind(uuid::Uuid::new_v4().to_string())
        .bind(traveler_id)
        .bind(conversation_id)
        .bind(note)
        .execute(&mut *tx)
        .await
        .map_err(AppError::Database)?;
    }

    // Title the conversation on its FIRST turn only; later turns just bump
    // updated_at. The local title is instant and can be upgraded later.
    let current_title: Option<String> = sqlx::query_scalar(
        "SELECT title FROM chat_conversations WHERE id = ?1",
    )
    .bind(conversation_id)
    .fetch_optional(&mut *tx)
    .await?;

    let local_title = matches!(
        current_title.as_deref(),
        None | Some("") | Some("New chat")
    )
    .then(|| title_from(user_message));

    if let Some(title) = &local_title {
        sqlx::query(
            "UPDATE chat_conversations SET title = ?1, updated_at = datetime('now') WHERE id = ?2",
        )
        .bind(title)
        .bind(conversation_id)
        .execute(&mut *tx)
        .await
        .map_err(AppError::Database)?;
    } else {
        sqlx::query("UPDATE chat_conversations SET updated_at = datetime('now') WHERE id = ?1")
            .bind(conversation_id)
            .execute(&mut *tx)
            .await
            .map_err(AppError::Database)?;
    }

    tx.commit().await.map_err(AppError::Database)?;
    Ok(local_title)
}

/// Replace a locally derived title with the model's three-word one, but only
/// while the local title is still in place — a user rename must win. Runs after
/// `Done`, so the extra model call is never on the reply path.
pub async fn upgrade_title(
    pool: &SqlitePool,
    ai: &AiClient,
    traveler_id: &str,
    conversation_id: &str,
    user_message: &str,
    local_title: &str,
) -> Result<(), AppError> {
    let title = generate_title(ai, user_message).await;
    if title.trim().is_empty() || title == local_title {
        return Ok(());
    }
    sqlx::query(
        "UPDATE chat_conversations SET title = ?1 \
         WHERE id = ?2 AND traveler_id = ?3 AND title = ?4",
    )
    .bind(&title)
    .bind(conversation_id)
    .bind(traveler_id)
    .bind(local_title)
    .execute(pool)
    .await
    .map_err(AppError::Database)?;
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
