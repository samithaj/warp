use std::path::Path;

use super::*;

const CWD: &str = "/Users/example/dev/poa-agent";

fn label(head: &str) -> Option<String> {
    label_from_transcript_head(head, Path::new(CWD))
}

fn tail_label(tail: &str) -> Option<String> {
    label_from_transcript_tail(tail, Path::new(CWD))
}

/// A golden transcript shaped like the real thing (Claude Code 2.1.221, the
/// version these record shapes were read off on this machine):
/// `ai-title` + its `agent-name` mirror up front, the head `cwd`/`slug`
/// carrier, the first prompt behind an injected `<command-name>` wrapper, and
/// a renamed `ai-title` appended at the very end the way `/rename` writes one.
const GOLDEN_TRANSCRIPT: &str = concat!(
    r#"{"type":"ai-title","aiTitle":"Initial working title","sessionId":"61f785ca-1c31-4671-a420-f89c47875750"}"#,
    "\n",
    r#"{"type":"agent-name","agentName":"Initial working title"}"#,
    "\n",
    r#"{"type":"mode","mode":"default","cwd":"/Users/example/dev/poa-agent","slug":"quietly-humming-otter"}"#,
    "\n",
    r#"{"type":"user","message":{"role":"user","content":"<command-name>/context</command-name>"}}"#,
    "\n",
    r#"{"type":"user","message":{"role":"user","content":"add retries to the ingest DAG"}}"#,
    "\n",
    r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"..."}]}}"#,
    "\n",
    r#"{"type":"ai-title","aiTitle":"Add retries to the ingest DAG"}"#,
    "\n",
    r#"{"type":"ai-title","aiTitle":"Ingest reliability work"}"#,
    "\n",
);

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
    // `encode_cwd` replaces every non-alphanumeric character with `-` per
    // Claude's convention.
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
fn project_dir_mangles_dots_underscores_and_spaces() {
    // The mangle is the whole discovery mechanism: get it wrong and a project
    // silently has no sessions at all. Verified against this machine's real
    // `~/.claude/projects` layout.
    let cases = [
        (
            "/Users/example/dev/poa-agent",
            "-Users-example-dev-poa-agent",
        ),
        (
            "/Users/example/dev/cse_market_analysis",
            "-Users-example-dev-cse-market-analysis",
        ),
        (
            "/Users/example/My Projects/app",
            "-Users-example-My-Projects-app",
        ),
        (
            "/Users/example/dev/app/.claude/worktrees/w1",
            "-Users-example-dev-app--claude-worktrees-w1",
        ),
    ];
    for (cwd, expected) in cases {
        let dir = claude_project_dir(Path::new(cwd)).expect("dir should derive");
        assert!(
            dir.ends_with(expected),
            "{cwd} encoded as {dir:?}, expected to end with {expected}"
        );
    }
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

#[test]
fn tail_rename_wins_over_the_head_title() {
    // The whole point of the tail read: `/rename` appends at EOF, so a
    // head-only reader would keep showing the session's original name.
    assert_eq!(
        tail_label(GOLDEN_TRANSCRIPT).as_deref(),
        Some("Ingest reliability work")
    );
    assert_eq!(
        label(GOLDEN_TRANSCRIPT).as_deref(),
        Some("Ingest reliability work"),
        "head tier also takes the last title it can see"
    );
}

#[test]
fn tail_reads_the_golden_transcript_end_to_end() {
    // Exercises the real seek-based path, not just the pure core: the tail
    // read starts mid-file, so the first line it sees is normally cut.
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir
        .path()
        .join("61f785ca-1c31-4671-a420-f89c47875750.jsonl");
    std::fs::write(&path, GOLDEN_TRANSCRIPT).unwrap();
    assert_eq!(
        resolve_label_from_transcript(&path, Path::new(CWD)).as_deref(),
        Some("Ingest reliability work")
    );
}

#[test]
fn head_title_names_a_transcript_whose_tail_window_has_none() {
    // A long tool loop after the naming turn pushes every title out of the
    // 64 KiB tail window; the 256 KiB head read is the tier that catches it.
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir
        .path()
        .join("61f785ca-1c31-4671-a420-f89c47875750.jsonl");
    let mut transcript = String::from(r#"{"type":"ai-title","aiTitle":"Rerank eval harness"}"#);
    transcript.push('\n');
    let filler = "x".repeat(4096);
    while transcript.len() < (TAIL_READ_BYTES as usize) * 2 {
        transcript.push_str(&format!(
            r#"{{"type":"assistant","message":{{"role":"assistant","content":[{{"type":"text","text":"{filler}"}}]}}}}"#
        ));
        transcript.push('\n');
    }
    assert!(
        transcript.len() < HEAD_READ_BYTES,
        "must stay inside the head window"
    );
    std::fs::write(&path, &transcript).unwrap();

    assert_eq!(
        read_tail(&path)
            .as_deref()
            .and_then(|tail| label_from_transcript_tail(tail, Path::new(CWD))),
        None,
        "no title record inside the tail window"
    );
    assert_eq!(
        resolve_label_from_transcript(&path, Path::new(CWD)).as_deref(),
        Some("Rerank eval harness")
    );
}

#[test]
fn agent_name_records_are_read_like_ai_titles() {
    let tail = concat!(
        r#"{"type":"agent-name","agentName":"Wire up the rail"}"#,
        "\n"
    );
    assert_eq!(tail_label(tail).as_deref(), Some("Wire up the rail"));
    assert_eq!(label(tail).as_deref(), Some("Wire up the rail"));
}

#[test]
fn injected_wrappers_and_caveats_are_skipped_for_the_next_real_prompt() {
    // These are interstitial, so the tier moves on rather than giving up:
    // there is always a real prompt behind them.
    let head = concat!(
        r#"{"type":"user","message":{"role":"user","content":"<command-name>/context</command-name>"}}"#,
        "\n",
        r#"{"type":"user","message":{"role":"user","content":"Caveat: The messages below were generated..."}}"#,
        "\n",
        r#"{"type":"user","message":{"role":"user","content":"port the scan mechanism"}}"#,
        "\n",
    );
    assert_eq!(label(head).as_deref(), Some("port the scan mechanism"));
}

#[test]
fn a_wrapper_only_transcript_yields_no_prompt_name() {
    let head = concat!(
        r#"{"type":"user","message":{"role":"user","content":"<local-command-stdout>ok</local-command-stdout>"}}"#,
        "\n",
        r#"{"type":"user","message":{"role":"user","content":"Caveat: replayed context"}}"#,
        "\n",
    );
    assert_eq!(label(head), None);
}

#[test]
fn leading_summary_records_never_become_the_name() {
    // A `summary` describes the *pre-compaction parent* conversation, so using
    // it would name this session after a different one.
    let head = concat!(
        r#"{"type":"summary","summary":"Refactor the ingestion pipeline","leafUuid":"abc"}"#,
        "\n",
        r#"{"type":"user","message":{"role":"user","content":"now do the rail"}}"#,
        "\n",
    );
    assert_eq!(label(head).as_deref(), Some("now do the rail"));
}

#[test]
fn slug_is_the_last_resort_and_is_de_kebabed() {
    let head = concat!(
        r#"{"type":"mode","mode":"default","slug":"quietly-humming-otter"}"#,
        "\n",
        r#"{"type":"user","message":{"role":"user","content":"<command-name>/clear</command-name>"}}"#,
        "\n",
    );
    assert_eq!(label(head).as_deref(), Some("quietly humming otter"));
}

#[test]
fn derived_style_junk_names_are_rejected_at_every_tier() {
    // `nameSource: "derived"` names look like `<dir>-<2hex>`; Claude's own docs
    // say they are display junk, not a handle.
    let head = concat!(
        r#"{"type":"ai-title","aiTitle":"poa-agent-3f"}"#,
        "\n",
        r#"{"type":"user","message":{"role":"user","content":"the actual request"}}"#,
        "\n",
    );
    assert_eq!(tail_label(head), None, "junk is rejected in the tail too");
    assert_eq!(label(head).as_deref(), Some("the actual request"));
}

#[test]
fn bare_hex_blobs_are_rejected() {
    // A session id or sha that reached a name slot is never a name.
    for junk in [
        r#"{"type":"ai-title","aiTitle":"3f8a9c2d"}"#,
        r#"{"type":"ai-title","aiTitle":"61f785ca-1c31"}"#,
        r#"{"type":"ai-title","aiTitle":"deadbeefcafe"}"#,
    ] {
        assert_eq!(tail_label(junk), None, "should reject: {junk}");
    }
    // Short hex-looking words are real words, and must survive.
    assert_eq!(
        tail_label(r#"{"type":"ai-title","aiTitle":"decaf"}"#).as_deref(),
        Some("decaf")
    );
}

#[test]
fn tail_skips_a_record_cut_by_the_seek() {
    // The first line of a tail read normally starts mid-record.
    let tail = concat!(
        r#"e":"ai-title","aiTitle":"Cut in half"}"#,
        "\n",
        r#"{"type":"ai-title","aiTitle":"Whole record"}"#,
        "\n",
    );
    assert_eq!(tail_label(tail).as_deref(), Some("Whole record"));
}
