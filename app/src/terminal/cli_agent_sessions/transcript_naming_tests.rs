use std::path::Path;

use super::*;

const CWD: &str = "/Users/example/dev/poa-agent";

fn label(head: &str) -> Option<String> {
    label_from_transcript_head(head, Path::new(CWD))
}

#[test]
fn last_ai_title_wins_over_earlier_ones_and_prompts() {
    let head = concat!(
        r#"{"type":"user","message":{"role":"user","content":"first prompt text"}}"#,
        "\n",
        r#"{"type":"ai-title","aiTitle":"Early working title"}"#,
        "\n",
        r#"{"type":"ai-title","aiTitle":"Add Jira search and fix orbit dashboard startup"}"#,
        "\n",
    );
    assert_eq!(
        label(head).as_deref(),
        Some("Add Jira search and fix orbit dashboard startup")
    );
}

#[test]
fn falls_back_to_first_user_prompt_without_ai_title() {
    let head = concat!(
        r#"{"type":"summary","summary":"unrelated"}"#,
        "\n",
        r#"{"type":"user","message":{"role":"user","content":"fix the retry backoff in the DAG"}}"#,
        "\n",
        r#"{"type":"user","message":{"role":"user","content":"second prompt ignored"}}"#,
        "\n",
    );
    assert_eq!(
        label(head).as_deref(),
        Some("fix the retry backoff in the DAG")
    );
}

#[test]
fn array_content_blocks_are_supported() {
    let head = concat!(
        r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"prompt from blocks"}]}}"#,
        "\n",
    );
    assert_eq!(label(head).as_deref(), Some("prompt from blocks"));
}

#[test]
fn sidechain_user_records_are_not_prompts() {
    let head = concat!(
        r#"{"type":"user","isSidechain":true,"message":{"role":"user","content":"subagent replay"}}"#,
        "\n",
        r#"{"type":"user","message":{"role":"user","content":"the real prompt"}}"#,
        "\n",
    );
    assert_eq!(label(head).as_deref(), Some("the real prompt"));
}

#[test]
fn junk_auto_name_is_rejected_and_falls_through() {
    // "poa-agent-0f" is Claude's own `<dir>-<2hex>` display default.
    let head = concat!(
        r#"{"type":"ai-title","aiTitle":"poa-agent-0f"}"#,
        "\n",
        r#"{"type":"user","message":{"role":"user","content":"real work description"}}"#,
        "\n",
    );
    assert_eq!(label(head).as_deref(), Some("real work description"));
}

#[test]
fn bare_directory_name_is_rejected() {
    let head = concat!(r#"{"type":"ai-title","aiTitle":"poa-agent"}"#, "\n");
    assert_eq!(label(head), None);
}

#[test]
fn whitespace_only_candidates_are_rejected() {
    let head = concat!(
        r#"{"type":"ai-title","aiTitle":"   "}"#,
        "\n",
        r#"{"type":"user","message":{"role":"user","content":"  \n\t "}}"#,
        "\n",
    );
    assert_eq!(label(head), None);
}

#[test]
fn long_labels_are_collapsed_and_ellipsized() {
    let long = "word ".repeat(60);
    let head = format!(r#"{{"type":"user","message":{{"role":"user","content":"{long}"}}}}"#);
    let resolved = label(&head).expect("long prompt should still name the row");
    assert!(resolved.chars().count() <= 80);
    assert!(resolved.ends_with('…'));
    assert!(!resolved.contains("  "), "whitespace is collapsed");
}

#[test]
fn truncated_final_line_and_garbage_lines_are_skipped() {
    let head = concat!(
        r#"not json at all"#,
        "\n",
        r#"{"type":"ai-title","aiTitle":"Debug cargo install compilation error"}"#,
        "\n",
        // A record cut off mid-way by the bounded read.
        r#"{"type":"ai-title","aiTi"#,
    );
    assert_eq!(
        label(head).as_deref(),
        Some("Debug cargo install compilation error")
    );
}

#[test]
fn empty_and_missing_content_yield_none() {
    assert_eq!(label(""), None);
    assert_eq!(label("\n\n"), None);
    let head = concat!(r#"{"type":"user","message":{"role":"user"}}"#, "\n");
    assert_eq!(label(head), None);
}

#[test]
fn transcript_path_matches_claude_layout() {
    // `encode_cwd` replaces both `/` and `.` with `-` per Claude's convention.
    let path = claude_transcript_path(
        Path::new("/Users/example/dev/poa-agent"),
        "61f785ca-1c31-4671-a420-f89c47875750",
    )
    .expect("path should derive");
    let path = path.to_string_lossy();
    assert!(
        path.ends_with(
            "projects/-Users-example-dev-poa-agent/61f785ca-1c31-4671-a420-f89c47875750.jsonl"
        ),
        "unexpected layout: {path}"
    );
}

#[test]
fn unreadable_file_resolves_to_none_not_error() {
    assert_eq!(
        resolve_label_from_transcript(
            Path::new("/nonexistent/definitely/missing.jsonl"),
            Path::new(CWD)
        ),
        None
    );
}
