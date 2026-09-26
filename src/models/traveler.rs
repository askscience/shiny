use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Traveler {
    pub id: String,
    pub name: String,
    pub email: String,
    pub password_hash: String,
    pub auth_token: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub username: Option<String>,
    pub avatar: Option<String>,
    #[sqlx(default)]
    pub is_admin: Option<i64>,
    /// Real Linux login name this account is bound to (Linux-user mode).
    #[sqlx(default)]
    pub unix_user: Option<String>,
    #[sqlx(default)]
    pub unix_uid: Option<i64>,
    #[sqlx(default)]
    pub unix_home: Option<String>,
    /// How the account authenticates: `local` (Argon2) or `pam`.
    #[sqlx(default)]
    pub auth_source: Option<String>,
}

impl Traveler {
    pub fn new(name: String, username: String, password_hash: String) -> Self {
        let email = format!("{username}@shiny.local");
        Self {
            id: Uuid::new_v4().to_string(),
            name,
            email,
            password_hash,
            auth_token: None,
            created_at: None,
            updated_at: None,
            username: Some(username),
            avatar: None,
            is_admin: Some(0),
            unix_user: None,
            unix_uid: None,
            unix_home: None,
            auth_source: Some("local".into()),
        }
    }

    pub fn to_public(&self) -> TravelerPublic {
        TravelerPublic {
            id: self.id.clone(),
            name: self.name.clone(),
            username: self.username.clone().unwrap_or_default(),
            avatar: self.avatar.clone(),
            created_at: self.created_at.clone(),
            is_admin: self.is_admin.unwrap_or(0) == 1,
            unix_user: self.unix_user.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TravelerPublic {
    pub id: String,
    pub name: String,
    pub username: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar: Option<String>,
    pub created_at: Option<String>,
    #[serde(default)]
    pub is_admin: bool,
    /// Real Linux login name when the account is bound to an OS user.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unix_user: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RegisterRequest {
    pub username: String,
    pub password: String,
    #[serde(default)]
    pub avatar: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct AuthResponse {
    pub token: String,
    pub traveler: TravelerPublic,
}

#[derive(Debug, Deserialize)]
pub struct UpdateTravelerRequest {
    pub name: Option<String>,
    pub username: Option<String>,
    pub avatar: Option<String>,
}
