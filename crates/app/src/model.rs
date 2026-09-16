//! View-model types for the UI. Everything the table renders is pre-formatted into
//! `SharedString`s so `render_td` never allocates.

use std::sync::Arc;

use chrono::{Days, Local};
use gpui_kit::*;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Cli,
    Sdk,
    Background,
    History,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Cli => "CLI",
            Kind::Sdk => "SDK (Agent SDK)",
            Kind::Background => "BG (background)",
            Kind::History => "History (record cleaned up)",
        }
    }
}

/// One session row. Display strings are pre-formatted so `render_td` does no work.
pub struct SessionRow {
    pub num: usize,
    pub num_fmt: SharedString,
    pub short_id: SharedString,
    pub uuid: SharedString,
    pub project: SharedString,
    pub title: SharedString,
    pub first_prompt: SharedString,
    pub last_prompt: SharedString,
    pub last_message: SharedString,
    pub started_at: i64,
    pub updated_at: i64,
    pub started_fmt: SharedString,
    pub updated_fmt: SharedString,
    pub started_full: SharedString,
    pub updated_full: SharedString,
    pub prompts: u32,
    pub prompts_fmt: SharedString,
    pub size: u64,
    pub size_fmt: SharedString,
    pub cost: f64,
    pub cost_fmt: SharedString,
    pub branch: SharedString,
    pub tools: u32,
    pub tools_fmt: SharedString,
    pub kind: Kind,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum DisplayMode {
    Title,
    FirstPrompt,
    LastPrompt,
    LastMessage,
}

impl DisplayMode {
    pub const ALL: [DisplayMode; 4] = [
        DisplayMode::Title,
        DisplayMode::FirstPrompt,
        DisplayMode::LastPrompt,
        DisplayMode::LastMessage,
    ];

    pub fn label(self) -> &'static str {
        match self {
            DisplayMode::Title => "Title",
            DisplayMode::FirstPrompt => "First prompt",
            DisplayMode::LastPrompt => "Last prompt",
            DisplayMode::LastMessage => "Last message",
        }
    }

    pub fn shortcut(self) -> &'static str {
        match self {
            DisplayMode::Title => "⌘1",
            DisplayMode::FirstPrompt => "⌘2",
            DisplayMode::LastPrompt => "⌘3",
            DisplayMode::LastMessage => "⌘4",
        }
    }

    pub fn preview(self, row: &SessionRow) -> SharedString {
        match self {
            DisplayMode::Title => row.title.clone(),
            DisplayMode::FirstPrompt => row.first_prompt.clone(),
            DisplayMode::LastPrompt => row.last_prompt.clone(),
            DisplayMode::LastMessage => row.last_message.clone(),
        }
    }
}

/// Search scope: which message roles a query runs against.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ScopeSet {
    pub user: bool,
    pub assistant: bool,
    pub tools: bool,
}

impl Default for ScopeSet {
    fn default() -> Self {
        Self { user: true, assistant: true, tools: false }
    }
}

impl ScopeSet {
    pub const ITEMS: [(&'static str, &'static str); 3] =
        [("User", "prompts"), ("Assistant", "replies"), ("Tools", "regex only")];

    pub fn get(&self, ix: usize) -> bool {
        match ix {
            0 => self.user,
            1 => self.assistant,
            _ => self.tools,
        }
    }

    pub fn toggle(&mut self, ix: usize) {
        match ix {
            0 => self.user = !self.user,
            1 => self.assistant = !self.assistant,
            _ => self.tools = !self.tools,
        }
    }

    pub fn label(&self) -> String {
        match (self.user, self.assistant, self.tools) {
            (true, true, true) => "All".into(),
            (true, true, false) => "Both".into(),
            (false, false, false) => "None".into(),
            _ => {
                let mut on = Vec::new();
                if self.user {
                    on.push("User");
                }
                if self.assistant {
                    on.push("Assistant");
                }
                if self.tools {
                    on.push("Tools");
                }
                on.join(", ")
            }
        }
    }
}

/// Updated-time filter presets.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum RangePreset {
    Today,
    Days3,
    Days7,
    Days30,
    Any,
}

impl RangePreset {
    pub const ALL: [RangePreset; 5] =
        [RangePreset::Today, RangePreset::Days3, RangePreset::Days7, RangePreset::Days30, RangePreset::Any];

    pub fn label(self) -> &'static str {
        match self {
            RangePreset::Today => "Today",
            RangePreset::Days3 => "3 days",
            RangePreset::Days7 => "7 days",
            RangePreset::Days30 => "30 days",
            RangePreset::Any => "Any time",
        }
    }

    /// Lower bound (ms since epoch) for `updated_at`, or `None` for no filter.
    pub fn since_ms(self) -> Option<i64> {
        let days = match self {
            RangePreset::Today => 0,
            RangePreset::Days3 => 2,
            RangePreset::Days7 => 6,
            RangePreset::Days30 => 29,
            RangePreset::Any => return None,
        };
        let today = Local::now().date_naive();
        let start = today.checked_sub_days(Days::new(days)).unwrap_or(today);
        let start = start.and_hms_opt(0, 0, 0)?.and_local_timezone(Local).single()?;
        Some(start.timestamp_millis())
    }
}

/// Filter applied to the table rows before sorting.
#[derive(Clone, Default, PartialEq)]
pub struct Filter {
    pub project: Option<SharedString>,
    pub since_ms: Option<i64>,
}

impl Filter {
    pub fn matches(&self, row: &SessionRow) -> bool {
        let project_ok = self.project.as_ref().is_none_or(|p| row.project == *p);
        let since_ok = self.since_ms.is_none_or(|since| row.updated_at >= since);
        project_ok && since_ok
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MessageKind {
    User,
    Assistant,
    Thinking,
    Tool,
}

/// One entry of a session transcript, pre-formatted for the conversation view.
pub struct Message {
    pub kind: MessageKind,
    pub time: SharedString,
    pub text: SharedString,
    /// Tool name (tool messages only).
    pub name: SharedString,
    /// Tool argument summary (tool messages only).
    pub arg: SharedString,
    /// Tool result summary, e.g. `212 lines` (tool messages only).
    pub result: SharedString,
    pub input: SharedString,
    pub output: SharedString,
    pub more: SharedString,
    /// Thinking length summary, e.g. `1.2k chars`.
    pub meta: SharedString,
    /// First 60 chars of thinking, shown while collapsed.
    pub preview: SharedString,
}

pub type Conversation = Vec<Arc<Message>>;
