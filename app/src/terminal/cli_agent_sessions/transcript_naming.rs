//! Conversation-name resolution from a Claude Code transcript.
//!
//! The project rail names a task by its conversation, not its directory. For a
//! dormant task (agent exited, or the app restarted) the only surviving source
//! is the transcript on disk, so this module derives a display label from it,
//! first acceptable candidate wins:
//!
//! 1. The **last** `{"type":"ai-title","aiTitle":…}` (or its
//!    `{"type":"agent-name","agentName":…}` sibling) in a **tail** read —
//!    Claude Code appends a fresh record every turn, and `/rename` appends one
//!    at the very end, so only reading from the end can see a rename. This is
//!    the tier that makes a renamed session show its new name.
//! 2. The last `ai-title` inside the **head** read — for a transcript longer
//!    than the tail budget whose titles all sit early (a session that was
//!    named and then ran a very long tool loop).
//! 3. The **first** real user prompt, truncated — the guaranteed floor.
//! 4. The transcript's `slug`, de-kebabed — last resort before no name at all.
//!
//! Junk names are rejected rather than displayed: Claude's auto-generated
//! `<dir>-<2hex>` display name (its own docs say it is not a resume handle),
//! bare hex blobs, whitespace, and the truncated cwd that produced the
//! original "six rows all reading `..uellig/repos/poa-agent`" bug.
//!
//! Reads are **bounded** (64 KiB tail + 256 KiB head) and belong off the
//! render path: callers resolve in a spawned task and cache the result
//! (`AgentSessionHandleOp::SetTitle`, or `session_scan`'s per-mtime memo);
//! nothing here may run inside element layout. The corpus is multi-gigabyte —
//! no code path may ever read a whole transcript.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::ai::agent_sdk::driver::harness::claude_transcript::{claude_config_dir, encode_cwd};

/// Head-read budget. Large enough that the first prompt and the first
/// `ai-title` are always inside it; small enough to be harmless on a slow
/// disk. (Matches the bound Orbit settled on for the same job.)
const HEAD_READ_BYTES: usize = 256 * 1024;

/// Tail-read budget. A `/rename` lands as the *last* `ai-title` record, and
/// Claude re-emits one every turn, so the newest name is always within a few
/// kilobytes of EOF; 64 KiB is slack for a turn carrying large tool output.
const TAIL_READ_BYTES: u64 = 64 * 1024;

/// Maximum length of a derived label; longer candidates are ellipsized.
const MAX_LABEL_LEN: usize = 80;

/// Prefix Claude injects ahead of replayed context. Never a name.
const CAVEAT_PREFIX: &str = "Caveat:";

/// Where Claude Code stores the transcript for `session_id` started in `cwd`:
/// `<config>/projects/<encoded-cwd>/<session_id>.jsonl`. Derived, not awaited:
/// the hook only reports `transcript_path` on `stop`, but `cwd` + session id
/// arrive on every event.
pub fn claude_transcript_path(cwd: &Path, session_id: &str) -> Option<PathBuf> {
    Some(claude_project_dir(cwd)?.join(format!("{session_id}.jsonl")))
}

/// The directory holding every transcript Claude Code recorded for `cwd`.
///
/// `cwd` is used as given; callers that have a real directory should
/// canonicalize first (Claude files a session under the resolved realpath, so
/// a symlinked checkout would otherwise miss). Canonicalization is I/O and is
/// deliberately left to the caller's background scan rather than baked in
/// here, where this is also called from paths that only have a remembered
/// string.
pub fn claude_project_dir(cwd: &Path) -> Option<PathBuf> {
    Some(
        claude_config_dir()
            .ok()?
            .join("projects")
            .join(encode_cwd(cwd)),
    )
}

/// Resolves a display label for the session behind `transcript_path`.
///
/// Returns `None` when the file is unreadable or yields no acceptable name —
/// a normal outcome (deleted transcript, tiny session), never an error. The
/// caller falls back to its own floor (`Agent · <short-id>`).
pub fn resolve_label_from_transcript(transcript_path: &Path, cwd: &Path) -> Option<String> {
    // Tail first: it is the only read that can see a `/rename`, and it is the
    // smaller of the two. A transcript shorter than the budget is read whole
    // by this one seek, so the head read below is skipped entirely.
    if let Some(label) = read_tail(transcript_path)
        .as_deref()
        .and_then(|tail| label_from_transcript_tail(tail, cwd))
    {
        return Some(label);
    }
    let head = read_head(transcript_path)?;
    label_from_transcript_head(&head, cwd)
}

/// Reads at most [`TAIL_READ_BYTES`] ending at EOF.
///
/// The *first* line of the slice is normally cut mid-record; the per-line
/// parse below skips it. No locking: Claude appends without one, so a torn
/// final line is expected and handled the same way.
fn read_tail(transcript_path: &Path) -> Option<String> {
    let mut file = File::open(transcript_path).ok()?;
    let len = file.metadata().ok()?.len();
    file.seek(SeekFrom::Start(len.saturating_sub(TAIL_READ_BYTES)))
        .ok()?;
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer).ok()?;
    // Lossy, not strict: the seek can land inside a multi-byte character.
    Some(String::from_utf8_lossy(&buffer).into_owned())
}

/// Reads at most [`HEAD_READ_BYTES`] from offset 0.
fn read_head(transcript_path: &Path) -> Option<String> {
    let mut buffer = vec![0_u8; HEAD_READ_BYTES];
    let read = File::open(transcript_path)
        .and_then(|mut file| file.read(&mut buffer))
        .ok()?;
    buffer.truncate(read);
    Some(String::from_utf8_lossy(&buffer).into_owned())
}

/// Pure core of the tail tier: the newest acceptable title in `tail`.
///
/// Walks backwards so a `/rename` — appended last — wins over the titles
/// Claude emits every turn, and so a long tail costs one reversed scan rather
/// than a full parse.
fn label_from_transcript_tail(tail: &str, cwd: &Path) -> Option<String> {
    tail.lines()
        .rev()
        .filter_map(title_record_text)
        .map(|candidate| tidy(&candidate))
        .find(|candidate| is_acceptable_label(candidate, cwd))
}

/// Pure core of the head tiers, separated for tests.
///
/// The final line of a bounded read is usually truncated mid-record; malformed
/// lines are skipped, never treated as errors.
fn label_from_transcript_head(head: &str, cwd: &Path) -> Option<String> {
    let mut last_ai_title: Option<String> = None;
    let mut first_user_prompt: Option<String> = None;
    let mut slug: Option<String> = None;

    for line in head.lines() {
        // Cheap substring pre-filters keep serde parsing off most lines.
        let looks_like_title = line.contains("\"ai-title\"") || line.contains("\"agent-name\"");
        let looks_like_user = first_user_prompt.is_none() && line.contains("\"user\"");
        let looks_like_slug = slug.is_none() && line.contains("\"slug\"");
        if !looks_like_title && !looks_like_user && !looks_like_slug {
            continue;
        }
        let Ok(record) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if slug.is_none()
            && let Some(text) = record.get("slug").and_then(Value::as_str)
        {
            // De-kebab: the slug is a filename-safe rendering of a phrase.
            slug = Some(text.replace('-', " "));
        }
        match record.get("type").and_then(Value::as_str) {
            // Deliberately the *last* title in the head, not the first: it
            // costs the same single pass and is strictly closer to the newest
            // name, which is what this tier is standing in for.
            Some("ai-title" | "agent-name") => {
                if let Some(title) = title_text(&record) {
                    last_ai_title = Some(title);
                }
            }
            Some("user") => {
                if first_user_prompt.is_none()
                    && let Some(text) = real_user_prompt_text(&record)
                {
                    first_user_prompt = Some(text);
                }
            }
            // A leading `summary` record describes the *pre-compaction parent*
            // conversation, not this one, so it is never a name source. Listed
            // rather than matched by wildcard so a new record type that does
            // carry a name forces a decision here.
            _ => {}
        }
    }

    [last_ai_title, first_user_prompt, slug]
        .into_iter()
        .flatten()
        .map(|candidate| tidy(&candidate))
        .find(|candidate| is_acceptable_label(candidate, cwd))
}

/// The title carried by a single transcript line, if it is a title record.
/// Used by the reversed tail walk, which has no state to accumulate.
fn title_record_text(line: &str) -> Option<String> {
    if !line.contains("\"ai-title\"") && !line.contains("\"agent-name\"") {
        return None;
    }
    let record = serde_json::from_str::<Value>(line).ok()?;
    match record.get("type").and_then(Value::as_str) {
        Some("ai-title" | "agent-name") => title_text(&record),
        _ => None,
    }
}

/// The name field of a title record. `agent-name` mirrors `ai-title`.
fn title_text(record: &Value) -> Option<String> {
    record
        .get("aiTitle")
        .or_else(|| record.get("agentName"))
        .and_then(Value::as_str)
        .map(str::to_owned)
}

/// Prompt text from a `user` record, or `None` when this record is not the
/// user's own words.
///
/// Rejected, so the scan moves on to the *next* user record rather than
/// giving up on the tier: subagent sidechain replays, Claude's injected
/// wrappers (`<command-name>…`, `<local-command-stdout>…`), and the
/// `Caveat:` preamble it prepends to replayed context. All three are
/// interstitial — there is always a real prompt after them.
/// Visible to the crate because the transcript-content digest behind session
/// search extracts the same text from the same records: what the user actually
/// typed, with sidechain replays and injected wrappers already rejected.
pub(crate) fn real_user_prompt_text(record: &Value) -> Option<String> {
    if record.get("isSidechain").and_then(Value::as_bool) == Some(true) {
        return None;
    }
    let text = user_prompt_text(record)?;
    let trimmed = text.trim();
    if trimmed.starts_with('<') || trimmed.starts_with(CAVEAT_PREFIX) {
        return None;
    }
    Some(text)
}

/// Extracts prompt text from a `user` record. `message.content` is either a
/// plain string or an array of content blocks with `text` fields.
/// Visible to the crate for the same reason as [`real_user_prompt_text`].
pub(crate) fn user_prompt_text(record: &Value) -> Option<String> {
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
/// display name, anything that is just the directory name again — the rail
/// exists to replace path-derived labels, not to relay them — and bare hex
/// blobs, which are ids leaking into a name slot.
fn is_acceptable_label(candidate: &str, cwd: &Path) -> bool {
    if candidate.is_empty() {
        return false;
    }
    if is_hex_blob(candidate) {
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

/// Whether a candidate is nothing but hex (ignoring hyphens) — a session id,
/// a commit sha or a uuid fragment that reached a name slot. Bounded at eight
/// digits so real words that happen to be hex ("added", "beef") still pass.
fn is_hex_blob(candidate: &str) -> bool {
    let digits = candidate.replace('-', "");
    digits.len() >= 8 && digits.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[cfg(test)]
#[path = "transcript_naming_tests.rs"]
mod tests;
