use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use tempfile::TempDir;

use super::*;

const SESSION_A: &str = "61f785ca-1c31-4671-a420-f89c47875750";
const SESSION_B: &str = "0c553412-72fa-4c15-889f-9c380392eb89";

/// A fixture Claude config root plus a project directory to scan.
///
/// The config root is injected rather than discovered, so these tests never
/// read the developer's real `~/.claude` and never mutate the environment.
struct Fixture {
    _root: TempDir,
    config_root: PathBuf,
    cwd: PathBuf,
    project_dir: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let root = TempDir::new().unwrap();
        let config_root = root.path().join(".claude");
        let cwd = root.path().join("poa-agent");
        fs::create_dir_all(&cwd).unwrap();
        // `project_dir_in` canonicalizes before encoding — on macOS the temp
        // dir is under `/var`, a symlink to `/private/var`, which is exactly
        // the realpath case that guards.
        let project_dir = project_dir_in(&config_root, &cwd).0;
        fs::create_dir_all(&project_dir).unwrap();
        Self {
            _root: root,
            config_root,
            cwd,
            project_dir,
        }
    }

    /// Writes a transcript shaped like the real thing: a working title, the
    /// first prompt, then the newest title appended at the end.
    fn write_session(&self, session_id: &str, title: &str, prompt: &str) -> PathBuf {
        let path = self.project_dir.join(format!("{session_id}.jsonl"));
        fs::write(
            &path,
            format!(
                concat!(
                    r#"{{"type":"ai-title","aiTitle":"Working title"}}"#,
                    "\n",
                    r#"{{"type":"user","message":{{"role":"user","content":"{prompt}"}}}}"#,
                    "\n",
                    r#"{{"type":"ai-title","aiTitle":"{title}"}}"#,
                    "\n",
                ),
                title = title,
                prompt = prompt,
            ),
        )
        .unwrap();
        path
    }

    fn scan(&self) -> DirectoryScan {
        scan_directory(&self.config_root, &self.cwd, &NameMemo::new())
    }
}

fn set_mtime(path: &PathBuf, seconds_since_epoch: u64) {
    let when = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(seconds_since_epoch);
    fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(when)
        .unwrap();
}

#[test]
fn uuid_filename_filter_accepts_only_transcripts() {
    assert_eq!(
        session_id_from_transcript_file_name(&format!("{SESSION_A}.jsonl")),
        Some(SESSION_A)
    );
    // The `<uuid>/` subdirectories (session memory, subagents) sit right next
    // to the transcripts; without the `.jsonl` suffix they would become rows
    // for sessions that cannot be resumed.
    assert_eq!(session_id_from_transcript_file_name(SESSION_A), None);
    assert_eq!(session_id_from_transcript_file_name("memory"), None);
    // Anchored: no prefix, no suffix, no partial ids.
    assert_eq!(
        session_id_from_transcript_file_name(&format!("prefix-{SESSION_A}.jsonl")),
        None
    );
    assert_eq!(
        session_id_from_transcript_file_name(&format!("{SESSION_A}-extra.jsonl")),
        None
    );
    assert_eq!(
        session_id_from_transcript_file_name("61f785ca-1c31-4671-a420.jsonl"),
        None
    );
    assert_eq!(session_id_from_transcript_file_name("notes.jsonl"), None);
    assert_eq!(session_id_from_transcript_file_name("summary.json"), None);
    // Uppercase is not the form Claude writes, so it is not accepted either.
    assert_eq!(
        session_id_from_transcript_file_name("61F785CA-1C31-4671-A420-F89C47875750.jsonl"),
        None
    );
}

#[test]
fn scan_names_every_session_in_a_project_directory() {
    let fixture = Fixture::new();
    fixture.write_session(SESSION_A, "Add retries to the ingest DAG", "add retries");
    fixture.write_session(SESSION_B, "Rerank eval harness", "rerank");

    let (sessions, memo) = fixture.scan();

    assert_eq!(sessions.len(), 2, "both transcripts should be listed");
    let labels: HashMap<&str, Option<&str>> = sessions
        .iter()
        .map(|session| (session.session_id.as_str(), session.label.as_deref()))
        .collect();
    assert_eq!(
        labels.get(SESSION_A),
        Some(&Some("Add retries to the ingest DAG")),
        "the newest title wins, via the tail read"
    );
    assert_eq!(labels.get(SESSION_B), Some(&Some("Rerank eval harness")));
    // The cwd is echoed back as asked, not canonicalized, so the rail buckets
    // scanned rows by exactly the path it buckets tabs by.
    assert!(
        sessions
            .iter()
            .all(|session| session.cwd == fixture.cwd.to_string_lossy())
    );
    assert_eq!(memo.len(), 2, "successful reads are memoised");
}

#[test]
fn scan_ignores_subdirectories_and_foreign_files() {
    let fixture = Fixture::new();
    fixture.write_session(SESSION_A, "The only real session", "go");
    // Exactly what a live project directory looks like around a transcript.
    fs::create_dir_all(fixture.project_dir.join(SESSION_B).join("subagents")).unwrap();
    fs::create_dir_all(fixture.project_dir.join("memory")).unwrap();
    fs::write(fixture.project_dir.join("notes.jsonl"), "{}\n").unwrap();

    let (sessions, _) = fixture.scan();

    assert_eq!(
        sessions
            .iter()
            .map(|session| session.session_id.as_str())
            .collect::<Vec<_>>(),
        vec![SESSION_A]
    );
}

#[test]
fn scan_orders_newest_first() {
    let fixture = Fixture::new();
    let older = fixture.write_session(SESSION_A, "Older work", "a");
    let newer = fixture.write_session(SESSION_B, "Newer work", "b");
    set_mtime(&older, 1_700_000_000);
    set_mtime(&newer, 1_700_000_060);

    let (sessions, _) = fixture.scan();

    assert_eq!(
        sessions
            .iter()
            .map(|session| session.session_id.as_str())
            .collect::<Vec<_>>(),
        vec![SESSION_B, SESSION_A]
    );
}

#[test]
fn memoised_names_are_not_re_read() {
    let fixture = Fixture::new();
    let path = fixture.write_session(SESSION_A, "On disk", "a");
    set_mtime(&path, 1_700_000_000);
    let modified = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);

    // Seeded with a value the transcript does not contain: if it comes back,
    // the file was not read.
    let mut memo = NameMemo::new();
    memo.insert((path.clone(), modified), "Name from the memo".to_owned());
    let (sessions, returned) = scan_directory(&fixture.config_root, &fixture.cwd, &memo);

    assert_eq!(sessions[0].label.as_deref(), Some("Name from the memo"));
    // The returned memo is complete, not incremental — that is what lets the
    // model replace the directory's bucket instead of growing it forever.
    assert_eq!(returned.len(), 1);
    assert_eq!(
        returned.get(&(path.clone(), modified)).map(String::as_str),
        Some("Name from the memo")
    );

    // A changed mtime invalidates the entry, because the key includes it.
    set_mtime(&path, 1_700_000_999);
    let (sessions, returned) = scan_directory(&fixture.config_root, &fixture.cwd, &memo);
    assert_eq!(sessions[0].label.as_deref(), Some("On disk"));
    assert_eq!(returned.len(), 1, "the stale key is not carried forward");
    assert!(
        !returned.contains_key(&(path, modified)),
        "the superseded entry is dropped, so the memo cannot grow without bound"
    );
}

#[test]
fn scan_of_an_unknown_directory_is_empty_not_an_error() {
    let fixture = Fixture::new();
    let never_used = fixture.cwd.parent().unwrap().join("never-used");
    fs::create_dir_all(&never_used).unwrap();

    let (sessions, memo) = scan_directory(&fixture.config_root, &never_used, &NameMemo::new());

    assert!(sessions.is_empty());
    assert!(memo.is_empty());
}

#[test]
fn scan_is_bounded_per_directory() {
    let fixture = Fixture::new();
    // More transcripts than the cap, each with a distinct mtime.
    for index in 0..MAX_SCANNED_SESSIONS_PER_DIR + 4 {
        let session_id = format!("61f785ca-1c31-4671-a420-f89c478{index:05}");
        let path = fixture.write_session(&session_id, &format!("Session {index}"), "go");
        set_mtime(&path, 1_700_000_000 + index as u64);
    }

    let (sessions, _) = fixture.scan();

    assert_eq!(sessions.len(), MAX_SCANNED_SESSIONS_PER_DIR);
    assert_eq!(
        sessions[0].label.as_deref(),
        Some(format!("Session {}", MAX_SCANNED_SESSIONS_PER_DIR + 3).as_str()),
        "the cap keeps the newest, not an arbitrary slice"
    );
}

#[test]
fn transcript_existence_is_checked_against_the_project_directory() {
    let fixture = Fixture::new();
    fixture.write_session(SESSION_A, "Present", "go");

    assert!(
        transcript_exists_in(&fixture.config_root, &fixture.cwd, SESSION_A),
        "the scanned session's transcript is still there"
    );
    assert!(
        !transcript_exists_in(&fixture.config_root, &fixture.cwd, SESSION_B),
        "a pruned session must not be offered for resume"
    );
    // A non-UUID id can never name a transcript, so it never passes.
    assert!(!transcript_exists_in(
        &fixture.config_root,
        &fixture.cwd,
        "../../etc/passwd"
    ));
}
