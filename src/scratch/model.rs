//! Scratch pad domain types — plain-text capture pads.

use chrono::{DateTime, Utc};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct ScratchPad {
    pub id: Uuid,
    /// Plain text, rendered as-is. The first line doubles as the card title.
    pub body: String,
    pub pinned: bool,
    /// 6-color cycle, matches group dots (0..=5).
    pub color_idx: i64,
    pub trashed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct ScratchPadInput {
    pub body: String,
    pub pinned: bool,
    pub color_idx: i64,
}

impl ScratchPad {
    /// First non-empty line — the card title (spec §8.1).
    pub fn title(&self) -> &str {
        self.body
            .lines()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("(empty pad)")
    }
}
