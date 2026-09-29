//! Todo domain model.

use std::fmt;
use std::str::FromStr;

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::TodoError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Status {
    Open,
    InProgress,
    Done,
    Cancelled,
}

impl Status {
    pub fn as_str(&self) -> &'static str {
        match self {
            Status::Open => "open",
            Status::InProgress => "in_progress",
            Status::Done => "done",
            Status::Cancelled => "cancelled",
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(self, Status::Done | Status::Cancelled)
    }

    pub fn label(&self) -> &'static str {
        match self {
            Status::Open => "Open",
            Status::InProgress => "In progress",
            Status::Done => "Done",
            Status::Cancelled => "Cancelled",
        }
    }
}

impl FromStr for Status {
    type Err = TodoError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "open" => Ok(Status::Open),
            "in_progress" => Ok(Status::InProgress),
            "done" => Ok(Status::Done),
            "cancelled" => Ok(Status::Cancelled),
            _ => Err(TodoError::UnknownStatus(s.to_string())),
        }
    }
}

/// 0 = low, 1 = normal, 2 = high, 3 = urgent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Priority(u8);

impl Priority {
    pub const LOW: Priority = Priority(0);
    pub const NORMAL: Priority = Priority(1);
    pub const HIGH: Priority = Priority(2);
    pub const URGENT: Priority = Priority(3);

    pub fn new(v: u8) -> Result<Priority, TodoError> {
        if v <= 3 {
            Ok(Priority(v))
        } else {
            Err(TodoError::InvalidPriority(v))
        }
    }

    pub fn value(&self) -> u8 {
        self.0
    }

    pub fn label(&self) -> &'static str {
        match self.0 {
            0 => "Low",
            1 => "Normal",
            2 => "High",
            _ => "Urgent",
        }
    }
}

impl Default for Priority {
    fn default() -> Self {
        Priority::NORMAL
    }
}

impl fmt::Display for Priority {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Todo {
    pub id: Uuid,
    pub group_id: Uuid,
    pub parent_id: Option<Uuid>,
    pub title: String,
    pub notes: String,
    pub status: Status,
    pub priority: Priority,
    pub position: i64,
    pub due_date: Option<NaiveDate>,
    pub deleted_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub completed_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TodoGroup {
    pub id: Uuid,
    pub name: String,
    pub position: i64,
    pub created_at: String,
    pub updated_at: String,
}

/// Nested todo for tree rendering (live items only).
#[derive(Debug, Clone, PartialEq)]
pub struct TodoNode {
    pub todo: Todo,
    pub children: Vec<TodoNode>,
}

impl TodoNode {
    pub fn open_descendant_count(&self) -> usize {
        let mut n = usize::from(!self.todo.status.is_terminal());
        for c in &self.children {
            n += c.open_descendant_count();
        }
        n
    }
}
