//! Mail plugin — mail client backed by pimalaya `io-email` (IMAP + SMTP).

pub mod mail;
pub mod plugin;
pub mod routes;
pub mod tools;

pub use plugin::MailPlugin;
