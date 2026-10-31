use axum::extract::{State, Extension, Query};
use axum::Json;
use serde::{Serialize, Deserialize};

use crate::api::AppState;
use crate::errors::AppError;
use crate::models::{Traveler, DiaryEntry};

#[derive(Deserialize)]
pub struct ChatMessageBody {
    pub message: String,
}

#[derive(Serialize)]
pub struct ChatResponse {
    pub success: bool,
    pub reply: String,
    pub diary_context_used: bool,
}

#[derive(Serialize)]
pub struct ChatHistoryResponse {
    pub success: bool,
    pub data: Vec<ChatHistoryEntry>,
}

#[derive(Serialize)]
pub struct ChatHistoryEntry {
    pub role: String,
    pub content: String,
    pub timestamp: Option<String>,
    /// The assistant's reasoning trace for a thinking model, when one was
    /// recorded with the reply. Absent on every other row.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking: Option<String>,
}

#[derive(Deserialize)]
pub struct HistoryQuery {
    pub limit: Option<i64>,
}

#[derive(Deserialize)]
pub struct ConversationListQuery {
    pub limit: Option<i64>,
}

#[derive(Deserialize)]
pub struct ConversationQuery {
    pub limit: Option<i64>,
    /// Load messages older than this message id.
    pub before: Option<String>,
    /// Omit stored reasoning traces unless the caller explicitly asks for them.
    pub thinking: Option<bool>,
}

pub async fn send_message(
    State(state): State<AppState>,
    Extension(traveler): Extension<Traveler>,
    Json(body): Json<ChatMessageBody>,
) -> Result<Json<ChatResponse>, AppError> {
    sqlx::query(
        "INSERT INTO chat_messages (id, traveler_id, role, content, timestamp) \
         VALUES (?1, ?2, 'user', ?3, datetime('now'))",
    )
    .bind(uuid::Uuid::new_v4().to_string())
    .bind(&traveler.id)
    .bind(&body.message)
    .execute(&state.pool)
    .await
    .map_err(AppError::Database)?;

    let recent_diary = sqlx::query_as::<_, DiaryEntry>(
        "SELECT * FROM diary_entries WHERE traveler_id = ?1 \
         ORDER BY date DESC LIMIT 5",
    )
    .bind(&traveler.id)
    .fetch_all(&state.pool)
    .await
    .map_err(AppError::Database)?;

    let diary_context = if !recent_diary.is_empty() {
        let mut context = String::from("Here are the traveler's recent diary entries:\n\n");
        for entry in &recent_diary {
            context.push_str(&format!(
                "--- {} ---\n{}\n\n",
                entry.date,
                entry.content_markdown
            ));
        }
        context
    } else {
        String::from("The traveler has no diary entries yet.")
    };

    let system_prompt = format!(
        "You are a helpful travel companion AI. You have access to the traveler's diary. \
         Answer questions based on their travel history and diary entries.\n\n{}",
        diary_context
    );

    // The user message was inserted above, so this window (most recent 20,
    // reversed back into order) already contains it — no duplicate append.
    // Mid-turn UI rows (approval, tool, media) are JSON records for the chat,
    // not dialogue: the model only ever sees user/assistant turns.
    let recent = sqlx::query_as::<_, (String, String, Option<String>)>(
        "SELECT role, content, timestamp FROM chat_messages WHERE traveler_id = ?1 \
         AND role IN ('user', 'assistant') \
         ORDER BY timestamp DESC, rowid DESC LIMIT 20",
    )
    .bind(&traveler.id)
    .fetch_all(&state.pool)
    .await
    .map_err(AppError::Database)?;

    let mut messages = Vec::new();
    messages.push(("system".to_string(), system_prompt));

    for (role, content, _ts) in recent.iter().rev() {
        messages.push((role.clone(), content.clone()));
    }

    let ai = state.resolve_ai(&traveler.id).await;
    let reply = ai.client.chat(messages, ai.model.as_deref()).await?;

    sqlx::query(
        "INSERT INTO chat_messages (id, traveler_id, role, content, timestamp) \
         VALUES (?1, ?2, 'assistant', ?3, datetime('now'))",
    )
    .bind(uuid::Uuid::new_v4().to_string())
    .bind(&traveler.id)
    .bind(&reply)
    .execute(&state.pool)
    .await
    .map_err(AppError::Database)?;

    Ok(Json(ChatResponse {
        success: true,
        reply,
        diary_context_used: !recent_diary.is_empty(),
    }))
}

pub async fn history(
    State(state): State<AppState>,
    Extension(traveler): Extension<Traveler>,
    Query(params): Query<HistoryQuery>,
) -> Result<Json<ChatHistoryResponse>, AppError> {
    let limit = params.limit.unwrap_or(50).clamp(1, 500);

    let entries = sqlx::query_as::<_, (String, String, Option<String>)>(
        "SELECT role, content, timestamp FROM (\
             SELECT role, content, timestamp, rowid FROM chat_messages \
             WHERE traveler_id = ?1 ORDER BY timestamp DESC, rowid DESC LIMIT ?2\
         ) ORDER BY timestamp ASC, rowid ASC",
    )
    .bind(&traveler.id)
    .bind(limit)
    .fetch_all(&state.pool)
    .await
    .map_err(AppError::Database)?;

    let history: Vec<ChatHistoryEntry> = entries
        .into_iter()
        .map(|(role, content, ts)| ChatHistoryEntry {
            role,
            content,
            timestamp: ts,
            thinking: None,
        })
        .collect();

    Ok(Json(ChatHistoryResponse {
        success: true,
        data: history,
    }))
}

/* ── Conversations (resumable chat history) ─────────────────── */

#[derive(Serialize)]
pub struct ConversationSummary {
    pub id: String,
    pub title: String,
    pub updated_at: Option<String>,
    pub preview: Option<String>,
}

#[derive(Serialize)]
pub struct ConversationListResponse {
    pub success: bool,
    pub data: Vec<ConversationSummary>,
}

pub async fn list_conversations(
    State(state): State<AppState>,
    Extension(traveler): Extension<Traveler>,
    Query(params): Query<ConversationListQuery>,
) -> Result<Json<ConversationListResponse>, AppError> {
    let limit = params.limit.unwrap_or(100).clamp(1, 500);
    let rows = sqlx::query_as::<_, (String, String, Option<String>, Option<String>)>(
        "SELECT c.id, c.title, c.updated_at, \
                (SELECT content FROM chat_messages m \
                 WHERE m.conversation_id = c.id AND m.role IN ('user', 'assistant') \
                 ORDER BY m.timestamp DESC, m.rowid DESC LIMIT 1) AS preview \
         FROM chat_conversations c \
         WHERE c.traveler_id = ?1 \
         ORDER BY c.updated_at DESC \
         LIMIT ?2",
    )
    .bind(&traveler.id)
    .bind(limit)
    .fetch_all(&state.pool)
    .await
    .map_err(AppError::Database)?;

    let data = rows
        .into_iter()
        .map(|(id, title, updated_at, preview)| ConversationSummary {
            id,
            title,
            updated_at,
            preview,
        })
        .collect();

    Ok(Json(ConversationListResponse { success: true, data }))
}

#[derive(Serialize)]
pub struct ConversationCreated {
    pub id: String,
}

pub async fn create_conversation(
    State(state): State<AppState>,
    Extension(traveler): Extension<Traveler>,
) -> Result<Json<serde_json::Value>, AppError> {
    let id = uuid::Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO chat_conversations (id, traveler_id, title, created_at, updated_at) \
         VALUES (?1, ?2, 'New chat', datetime('now'), datetime('now'))",
    )
    .bind(&id)
    .bind(&traveler.id)
    .execute(&state.pool)
    .await
    .map_err(AppError::Database)?;

    Ok(Json(serde_json::json!({
        "success": true,
        "data": { "id": id },
    })))
}

#[derive(Deserialize)]
pub struct ConversationPath {
    pub id: String,
}

pub async fn conversation_messages(
    State(state): State<AppState>,
    Extension(traveler): Extension<Traveler>,
    axum::extract::Path(path): axum::extract::Path<ConversationPath>,
    Query(params): Query<ConversationQuery>,
) -> Result<Json<ChatHistoryResponse>, AppError> {
    let limit = params.limit.unwrap_or(500).clamp(1, 2000);
    let include_thinking = if params.thinking.unwrap_or(true) { 1 } else { 0 };
    let entries = sqlx::query_as::<_, (String, String, Option<String>, Option<String>)>(
        // `system` rows are invisible notes written for the model (e.g. "the
        // user stopped this reply"). They belong in the agent's history and
        // must never show up as a bubble in the user's chat.
        //
        // Newest-first window + reverse below returns the latest page in
        // chronological order; `before` walks older pages.
        "SELECT role, content, timestamp, \
                CASE WHEN ?3 THEN thinking ELSE NULL END AS thinking \
         FROM chat_messages \
         WHERE conversation_id = ?1 AND traveler_id = ?2 AND role != 'system' \
           AND (?4 IS NULL OR rowid < (\
               SELECT rowid FROM chat_messages \
               WHERE id = ?4 AND conversation_id = ?1 AND traveler_id = ?2\
           )) \
         ORDER BY timestamp DESC, rowid DESC \
         LIMIT ?5",
    )
    .bind(&path.id)
    .bind(&traveler.id)
    .bind(include_thinking)
    .bind(params.before.as_deref())
    .bind(limit)
    .fetch_all(&state.pool)
    .await
    .map_err(AppError::Database)?;

    let data: Vec<ChatHistoryEntry> = entries
        .into_iter()
        .rev()
        .map(|(role, content, timestamp, thinking)| ChatHistoryEntry {
            role,
            content,
            timestamp,
            thinking: thinking.filter(|t| !t.trim().is_empty()),
        })
        .collect();

    Ok(Json(ChatHistoryResponse { success: true, data }))
}

pub async fn delete_conversation(
    State(state): State<AppState>,
    Extension(traveler): Extension<Traveler>,
    axum::extract::Path(path): axum::extract::Path<ConversationPath>,
) -> Result<Json<serde_json::Value>, AppError> {
    sqlx::query("DELETE FROM chat_messages WHERE conversation_id = ?1 AND traveler_id = ?2")
        .bind(&path.id)
        .bind(&traveler.id)
        .execute(&state.pool)
        .await
        .map_err(AppError::Database)?;

    sqlx::query("DELETE FROM chat_conversations WHERE id = ?1 AND traveler_id = ?2")
        .bind(&path.id)
        .bind(&traveler.id)
        .execute(&state.pool)
        .await
        .map_err(AppError::Database)?;

    Ok(Json(serde_json::json!({ "success": true, "data": { "deleted": true } })))
}
