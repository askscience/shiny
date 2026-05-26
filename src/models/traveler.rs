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
}

impl Traveler {
    pub fn new(name: String, email: String, password_hash: String) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            name,
            email,
            password_hash,
            auth_token: None,
            created_at: None,
            updated_at: None,
        }
    }

    pub fn to_public(&self) -> TravelerPublic {
        TravelerPublic {
            id: self.id.clone(),
            name: self.name.clone(),
            email: self.email.clone(),
            created_at: self.created_at.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TravelerPublic {
    pub id: String,
    pub name: String,
    pub email: String,
    pub created_at: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RegisterRequest {
    pub name: String,
    pub email: String,
    pub password: String,
}

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub email: String,
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
    pub email: Option<String>,
}
