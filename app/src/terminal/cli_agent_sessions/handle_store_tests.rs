use super::*;

fn identify(session_id: &str, cwd: &str) -> AgentSessionHandleOp {
    AgentSessionHandleOp::Identify {
        agent: "Claude".to_owned(),
        pane_uuid: b"pane".to_vec(),
        cwd: cwd.to_owned(),
        session_id: session_id.to_owned(),
    }
}

#[test]
fn identify_inserts_at_front_and_reidentify_moves_not_duplicates() {
    let mut model = AgentSessionHandlesModel::default();
    assert!(model.apply_op(&identify("aaaa", "/dev/one")));
    assert!(model.apply_op(&identify("bbbb", "/dev/two")));
    assert_eq!(model.handles()[0].session_id, "bbbb");

    // Resuming a known session moves it to the front with the fresh cwd.
    assert!(model.apply_op(&identify("aaaa", "/dev/one-b")));
    assert_eq!(model.handles().len(), 2);
    assert_eq!(model.handles()[0].session_id, "aaaa");
    assert_eq!(model.handles()[0].cwd, "/dev/one-b");
}

#[test]
fn touch_moves_to_front_set_title_caches_forget_removes() {
    let mut model = AgentSessionHandlesModel::default();
    model.apply_op(&identify("aaaa", "/dev/one"));
    model.apply_op(&identify("bbbb", "/dev/two"));

    assert!(model.apply_op(&AgentSessionHandleOp::Touch {
        agent: "Claude".to_owned(),
        session_id: "aaaa".to_owned(),
    }));
    assert_eq!(model.handles()[0].session_id, "aaaa");

    model.apply_op(&AgentSessionHandleOp::SetTitle {
        agent: "Claude".to_owned(),
        session_id: "aaaa".to_owned(),
        title: "Fix retry backoff".to_owned(),
    });
    assert_eq!(
        model
            .get(CLIAgent::Claude, "aaaa")
            .unwrap()
            .title
            .as_deref(),
        Some("Fix retry backoff")
    );

    model.apply_op(&AgentSessionHandleOp::Forget {
        agent: "Claude".to_owned(),
        session_id: "aaaa".to_owned(),
    });
    assert!(model.get(CLIAgent::Claude, "aaaa").is_none());
    assert_eq!(model.handles().len(), 1);
}

#[test]
fn start_inflight_is_not_mirrored() {
    let mut model = AgentSessionHandlesModel::default();
    assert!(!model.apply_op(&AgentSessionHandleOp::StartInflight {
        agent: "Claude".to_owned(),
        pane_uuid: b"pane".to_vec(),
        cwd: "/dev/one".to_owned(),
    }));
    assert!(model.handles().is_empty());
}

#[test]
fn hydration_skips_unidentified_and_unknown_agent_rows() {
    let now = chrono::Utc::now().naive_utc();
    let record = |agent: &str, session_id: Option<&str>| AgentSessionHandleRecord {
        id: 0,
        agent: agent.to_owned(),
        session_id: session_id.map(str::to_owned),
        cwd: "/dev/one".to_owned(),
        pane_uuid: b"pane".to_vec(),
        title: None,
        created_at: now,
        last_seen_at: now,
    };
    let model = AgentSessionHandlesModel::from_records(&[
        record("Claude", Some("aaaa")),
        record("Claude", None),
        record("NotARealAgent", Some("bbbb")),
    ]);
    assert_eq!(model.handles().len(), 1);
    assert_eq!(model.handles()[0].session_id, "aaaa");
}

#[test]
fn identify_records_the_pane_uuid_for_restore_matching() {
    let mut model = AgentSessionHandlesModel::default();
    model.apply_op(&AgentSessionHandleOp::Identify {
        agent: "Claude".to_owned(),
        pane_uuid: b"pane-xyz".to_vec(),
        cwd: "/dev/one".to_owned(),
        session_id: "aaaa".to_owned(),
    });
    // The pane uuid is what a restored tab is matched on, so it must survive
    // into the mirror rather than being dropped with the op.
    assert_eq!(
        model.get(CLIAgent::Claude, "aaaa").unwrap().pane_uuid,
        b"pane-xyz".to_vec()
    );
}
