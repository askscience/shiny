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
}

#[derive(Deserialize)]
pub struct HistoryQuery {
    pub limit: Option<i64>,
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

    let chat_history = sqlx::query_as::<_, (String, String, Option<String>)>(
        "SELECT role, content, timestamp FROM chat_messages WHERE traveler_id = ?1 \
         ORDER BY timestamp ASC LIMIT 20",
    )
    .bind(&traveler.id)
    .fetch_all(&state.pool)
    .await
    .map_err(AppError::Database)?;

    let mut messages = Vec::new();
    messages.push(("system".to_string(), system_prompt));

    for (role, content, _ts) in &chat_history {
        messages.push((role.clone(), content.clone()));
    }

    messages.push(("user".to_string(), body.message.clone()));

    let reply = state.ollama.chat(messages, None).await?;

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
    let limit = params.limit.unwrap_or(50);

    let entries = sqlx::query_as::<_, (String, String, Option<String>)>(
        "SELECT role, content, timestamp FROM chat_messages WHERE traveler_id = ?1 \
         ORDER BY timestamp ASC LIMIT ?2",
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
) -> Result<Json<ConversationListResponse>, AppError> {
    let rows = sqlx::query_as::<_, (String, String, Option<String>, Option<String>)>(
        "SELECT c.id, c.title, c.updated_at, \
                (SELECT content FROM chat_messages m \
                 WHERE m.conversation_id = c.id ORDER BY m.timestamp DESC LIMIT 1) AS preview \
         FROM chat_conversations c \
         WHERE c.traveler_id = ?1 \
         ORDER BY c.updated_at DESC",
    )
    .bind(&traveler.id)
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
) -> Result<Json<ChatHistoryResponse>, AppError> {
    let entries = sqlx::query_as::<_, (String, String, Option<String>)>(
        "SELECT role, content, timestamp FROM chat_messages \
         WHERE conversation_id = ?1 AND traveler_id = ?2 \
         ORDER BY timestamp ASC",
    )
    .bind(&path.id)
    .bind(&traveler.id)
    .fetch_all(&state.pool)
    .await
    .map_err(AppError::Database)?;

    let data: Vec<ChatHistoryEntry> = entries
        .into_iter()
        .map(|(role, content, timestamp)| ChatHistoryEntry {
            role,
            content,
            timestamp,
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
