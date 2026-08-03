//! Conversation-name resolution from a Claude Code transcript.
//!
//! The project rail names a task by its conversation, not its directory. For a
//! dormant task (agent exited, or the app restarted) the only surviving source
//! is the transcript on disk, so this module derives a display label from it:
//!
//! 1. The **last** `{"type":"ai-title","aiTitle":…}` record — Claude Code
//!    writes AI-generated session titles (and `/rename` values) into the
//!    transcript as these records. Empirically present in ~90% of sessions
//!    with six or more messages.
//! 2. The **first** real user prompt, truncated — the guaranteed floor.
//!
//! Junk names are rejected rather than displayed: Claude's auto-generated
//! `<dir>-<2hex>` display name (its own docs say it is not a resume handle),
//! whitespace, and the truncated cwd that produced the original "six rows all
//! reading `..uellig/repos/poa-agent`" bug.
//!
//! Reads are **bounded** (256 KB head) and belong off the render path: callers
//! resolve in a spawned task and cache the result (`AgentSessionHandleOp::
//! SetTitle`); nothing here may run inside element layout.

use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::ai::agent_sdk::driver::harness::claude_transcript::{claude_config_dir, encode_cwd};

/// Head-read budget. Large enough that the first prompt and the first
/// `ai-title` are always inside it; small enough to be harmless on a slow
/// disk. (Matches the bound Orbit settled on for the same job.)
const HEAD_READ_BYTES: usize = 256 * 1024;

/// Maximum length of a derived label; longer candidates are ellipsized.
const MAX_LABEL_LEN: usize = 80;

/// Where Claude Code stores the transcript for `session_id` started in `cwd`:
/// `<config>/projects/<encoded-cwd>/<session_id>.jsonl`. Derived, not awaited:
/// the hook only reports `transcript_path` on `stop`, but `cwd` + session id
/// arrive on every event.
pub fn claude_transcript_path(cwd: &Path, session_id: &str) -> Option<PathBuf> {
    let config_root = claude_config_dir().ok()?;
    Some(
        config_root
            .join("projects")
            .join(encode_cwd(cwd))
            .join(format!("{session_id}.jsonl")),
    )
}

/// Resolves a display label for the session behind `transcript_path`.
///
/// Returns `None` when the file is unreadable or yields no acceptable name —
/// a normal outcome (deleted transcript, tiny session), never an error. The
/// caller falls back to its own floor (`Agent · <short-id>`).
pub fn resolve_label_from_transcript(transcript_path: &Path, cwd: &Path) -> Option<String> {
    let mut head = vec![0_u8; HEAD_READ_BYTES];
    let read = File::open(transcript_path)
        .and_then(|mut file| file.read(&mut head))
        .ok()?;
    head.truncate(read);
    let head = String::from_utf8_lossy(&head);
    label_from_transcript_head(&head, cwd)
}

/// Pure core of [`resolve_label_from_transcript`], separated for tests.
///
/// The final line of a bounded read is usually truncated mid-record; malformed
/// lines are skipped, never treated as errors.
fn label_from_transcript_head(head: &str, cwd: &Path) -> Option<String> {
    let mut last_ai_title: Option<String> = None;
    let mut first_user_prompt: Option<String> = None;

    for line in head.lines() {
        // Cheap substring pre-filters keep serde parsing off most lines.
        let looks_like_title = line.contains("\"ai-title\"");
        let looks_like_user = first_user_prompt.is_none() && line.contains("\"user\"");
        if !looks_like_title && !looks_like_user {
            continue;
        }
        let Ok(record) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        match record.get("type").and_then(Value::as_str) {
            Some("ai-title") => {
                if let Some(title) = record.get("aiTitle").and_then(Value::as_str) {
                    last_ai_title = Some(title.to_owned());
                }
            }
            Some("user") => {
                // Subagent sidechains replay user records that are not the
                // user's own prompt.
                if record.get("isSidechain").and_then(Value::as_bool) == Some(true) {
                    continue;
                }
                if first_user_prompt.is_none()
                    && let Some(text) = user_prompt_text(&record)
                {
                    first_user_prompt = Some(text);
                }
            }
            _ => {}
        }
    }

    [last_ai_title, first_user_prompt]
        .into_iter()
        .flatten()
        .map(|candidate| tidy(&candidate))
        .find(|candidate| is_acceptable_label(candidate, cwd))
}

/// Extracts prompt text from a `user` record. `message.content` is either a
/// plain string or an array of content blocks with `text` fields.
fn user_prompt_text(record: &Value) -> Option<String> {
    let content = record.get("message")?.get("content")?;
    match content {
        Value::String(text) => Some(text.clone()),
        Value::Array(blocks) => blocks
            .iter()
            .find_map(|block| block.get("text").and_then(Value::as_str))
            .map(str::to_owned),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::Object(_) => None,
    }
}

/// Collapses whitespace and ellipsizes to [`MAX_LABEL_LEN`].
fn tidy(candidate: &str) -> String {
    let collapsed = candidate.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= MAX_LABEL_LEN {
        return collapsed;
    }
    let truncated: String = collapsed.chars().take(MAX_LABEL_LEN - 1).collect();
    format!("{}…", truncated.trim_end())
}

/// Whether a candidate is worth showing over the caller's floor.
///
/// Rejects empty/whitespace, Claude's auto-generated `<cwd-basename>-<2hex>`
/// display name, and anything that is just the directory name again — the rail
/// exists to replace path-derived labels, not to relay them.
fn is_acceptable_label(candidate: &str, cwd: &Path) -> bool {
    if candidate.is_empty() {
        return false;
    }
    let Some(basename) = cwd.file_name().and_then(|name| name.to_str()) else {
        return true;
    };
    if candidate.eq_ignore_ascii_case(basename) {
        return false;
    }
    // Claude's junk default: "<basename>-<2 hex chars>", e.g. "poa-agent-0f".
    if let Some(suffix) = candidate
        .strip_prefix(basename)
        .and_then(|rest| rest.strip_prefix('-'))
        && suffix.len() == 2
        && suffix.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return false;
    }
    true
}

#[cfg(test)]
#[path = "transcript_naming_tests.rs"]
mod tests;
