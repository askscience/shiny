//! Calculator plugin — a basic + scientific calculator window and an agent
//! tool that evaluates math expressions with a shared, dependency-free engine.

pub mod eval;
pub mod plugin;
pub mod routes;
pub mod tools;

pub use plugin::CalculatorPlugin;
