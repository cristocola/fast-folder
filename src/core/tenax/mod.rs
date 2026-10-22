//! Tenax, the assistant in the guided app's sidebar: the protocol half.
//!
//! What a request to the Anthropic Messages API holds and how its answer is
//! read (`api`), who Tenax is (`persona`), what he may ask fastf to do
//! (`tools`), the guides he reads (`docs`), the loop that runs one turn of a
//! conversation (`turn`), and where the key lives and who may use which
//! (`key`). The app's half — the box, the chat, what the tools read, the
//! proposals a person answers — is `tui::app::tenax`.
//!
//! Nothing here prints, prompts or sleeps, and nothing here runs unless a
//! surface asks: `tenax = false` means no request is ever built.

pub mod api;
pub mod docs;
pub mod key;
pub mod persona;
pub mod tools;
pub mod turn;

pub use api::{ApiError, DEFAULT_MODEL};
pub use key::{Ask, KeySource, Secret};
