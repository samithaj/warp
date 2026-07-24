use std::path::Path;

use warp_util::standardized_path::StandardizedPath;

use super::super::project_key::ProjectKey;
use super::{ProjectEntry, ProjectId, ProjectLayout};

fn git_key(path: &str) -> ProjectKey {
    ProjectKey::LocalGit(StandardizedPath::try_from_local(Path::new(path)).unwrap())
}

/// Builds a layout directly from a per-tab project list, mirroring the
/// first-seen dedupe in `compute` (which itself needs a live app + pane groups
/// and is exercised by the GUI verification instead).
fn layout_from(tab_project: Vec<ProjectId>) -> ProjectLayout {
    let mut projects: Vec<ProjectEntry> = Vec::new();
    for id in &tab_project {
        if !projects.iter().any(|entry| &entry.id == id) {
            projects.push(ProjectEntry {
                display_name: id.display_name(),
                id: id.clone(),
            });
        }
    }
    ProjectLayout {
        projects,
        tab_project,
        tab_pane_group_ids: Vec::new(),
    }
}

#[test]
fn visible_tab_indices_selects_only_that_project() {
    let warp = ProjectId::Key(git_key("/Users/sam/dev/warp/.git"));
    let orbit = ProjectId::Key(git_key("/Users/sam/dev/orbit/.git"));
    let layout = layout_from(vec![
        warp.clone(),
        orbit.clone(),
        warp.clone(),
        ProjectId::Other,
    ]);
    assert_eq!(layout.visible_tab_indices(&warp), vec![0, 2]);
    assert_eq!(layout.visible_tab_indices(&orbit), vec![1]);
    assert_eq!(layout.visible_tab_indices(&ProjectId::Other), vec![3]);
}

#[test]
fn projects_are_distinct_in_first_seen_order() {
    let warp = ProjectId::Key(git_key("/Users/sam/dev/warp/.git"));
    let orbit = ProjectId::Key(git_key("/Users/sam/dev/orbit/.git"));
    let layout = layout_from(vec![orbit.clone(), warp.clone(), orbit.clone()]);
    let names: Vec<_> = layout
        .projects()
        .iter()
        .map(|entry| entry.display_name.clone())
        .collect();
    assert_eq!(names, vec!["orbit", "warp"]);
    assert!(layout.has_multiple_projects());
}

#[test]
fn other_bucket_is_named_other() {
    assert_eq!(ProjectId::Other.display_name(), "Other");
    let layout = layout_from(vec![ProjectId::Other]);
    assert!(!layout.has_multiple_projects());
}
