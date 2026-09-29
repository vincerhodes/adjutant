//! Entity identity for the generic link graph.
//!
//! Entity rows live in per-type tables (M1: `todos`); `entity_links`
//! references them loosely, so identity here is (kind, uuid) only.

use std::str::FromStr;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::link::LinkError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EntityType {
    Email,
    Thread,
    Event,
    Todo,
    Note,
    Reminder,
}

impl EntityType {
    pub fn as_str(&self) -> &'static str {
        match self {
            EntityType::Email => "email",
            EntityType::Thread => "thread",
            EntityType::Event => "event",
            EntityType::Todo => "todo",
            EntityType::Note => "note",
            EntityType::Reminder => "reminder",
        }
    }
}

impl FromStr for EntityType {
    type Err = LinkError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "email" => Ok(EntityType::Email),
            "thread" => Ok(EntityType::Thread),
            "event" => Ok(EntityType::Event),
            "todo" => Ok(EntityType::Todo),
            "note" => Ok(EntityType::Note),
            "reminder" => Ok(EntityType::Reminder),
            _ => Err(LinkError::UnknownEntityType(s.to_string())),
        }
    }
}

/// A reference to an entity row in a per-type table.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EntityRef {
    pub kind: EntityType,
    pub id: Uuid,
}

impl EntityRef {
    pub fn new(kind: EntityType, id: Uuid) -> Self {
        EntityRef { kind, id }
    }
}
