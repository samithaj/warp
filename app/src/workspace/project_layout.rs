//! The project-layout projection.
//!
//! Derives the set of projects from the currently-open tabs and maps each tab
//! to its project, so navigation and rendering can be scoped to a selected
//! project without ever filtering the raw `Workspace::tabs` vector in place.
//! This is the single source of truth the sidebar rail, the top tab bar, and
//! every navigation path consult.
//!
//! Recompute whenever tabs are added/removed/reordered or a tab's focused
//! repo/cwd changes; the result is a pure function of the tabs.

use std::path::PathBuf;

use warp_util::local_or_remote_path::LocalOrRemotePath;
use warpui::{AppContext, EntityId};

use super::project_key::ProjectKey;
use crate::tab::TabData;

/// Identifies a project bucket in the sidebar. `Other` collects tabs with no
/// detectable repo/directory (for example a bare home-directory session).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ProjectId {
    Key(ProjectKey),
    Other,
}

impl ProjectId {
    /// The rail label for this project.
    pub fn display_name(&self) -> String {
        match self {
            Self::Key(key) => key.display_name(),
            Self::Other => "Other".to_owned(),
        }
    }
}

/// One project entry shown in the rail.
#[derive(Debug, Clone)]
pub struct ProjectEntry {
    pub id: ProjectId,
    pub display_name: String,
}

/// A pure projection of the workspace's tabs into projects.
#[derive(Debug, Clone, Default)]
pub struct ProjectLayout {
    /// Distinct projects, in first-seen (stable) order.
    projects: Vec<ProjectEntry>,
    /// The project of each tab, parallel to `Workspace::tabs` by index.
    tab_project: Vec<ProjectId>,
    /// The pane-group id of each tab, parallel by index. Used to resolve a
    /// visible index back to a stable tab identity at the edge.
    tab_pane_group_ids: Vec<EntityId>,
}

impl ProjectLayout {
    /// Computes the projection from the workspace tabs.
    pub fn compute(tabs: &[TabData], ctx: &AppContext) -> Self {
        let mut projects: Vec<ProjectEntry> = Vec::new();
        let mut tab_project = Vec::with_capacity(tabs.len());
        let mut tab_pane_group_ids = Vec::with_capacity(tabs.len());

        for tab in tabs {
            let id = Self::project_of_tab_data(tab, ctx);
            if !projects.iter().any(|entry| entry.id == id) {
                projects.push(ProjectEntry {
                    display_name: id.display_name(),
                    id: id.clone(),
                });
            }
            tab_project.push(id);
            tab_pane_group_ids.push(tab.pane_group.id());
        }

        Self {
            projects,
            tab_project,
            tab_pane_group_ids,
        }
    }

    /// Resolves the project of a single tab from its focused session path.
    pub fn project_of_tab_data(tab: &TabData, ctx: &AppContext) -> ProjectId {
        let path: Option<PathBuf> = tab.pane_group.as_ref(ctx).active_session_path(ctx);
        path.and_then(|path| ProjectKey::for_path(&LocalOrRemotePath::Local(path), ctx))
            .map(ProjectId::Key)
            .unwrap_or(ProjectId::Other)
    }

    /// The distinct projects, in stable order.
    pub fn projects(&self) -> &[ProjectEntry] {
        &self.projects
    }

    /// The project of the tab at `index`, if in range.
    pub fn project_of_tab(&self, index: usize) -> Option<&ProjectId> {
        self.tab_project.get(index)
    }

    /// The pane-group id of the tab at `index`, if in range.
    pub fn pane_group_id_of_tab(&self, index: usize) -> Option<EntityId> {
        self.tab_pane_group_ids.get(index).copied()
    }

    /// Raw `Workspace::tabs` indices belonging to `selected`, in tab order.
    pub fn visible_tab_indices(&self, selected: &ProjectId) -> Vec<usize> {
        self.tab_project
            .iter()
            .enumerate()
            .filter_map(|(index, id)| (id == selected).then_some(index))
            .collect()
    }
}

/// The index reached by moving forward one step through `indices` from
/// `current`, wrapping at the end. If `current` is not in `indices`, returns
/// the first element (or `current` when empty). Used to cycle next-tab within a
/// project's visible tabs.
pub fn cycle_next(indices: &[usize], current: usize) -> usize {
    if indices.is_empty() {
        return current;
    }
    match indices.iter().position(|&index| index == current) {
        Some(pos) => indices[(pos + 1) % indices.len()],
        None => indices[0],
    }
}

/// The index reached by moving backward one step through `indices` from
/// `current`, wrapping at the start. If `current` is not in `indices`, returns
/// the first element (or `current` when empty).
pub fn cycle_prev(indices: &[usize], current: usize) -> usize {
    if indices.is_empty() {
        return current;
    }
    match indices.iter().position(|&index| index == current) {
        Some(pos) => indices[(pos + indices.len() - 1) % indices.len()],
        None => indices[0],
    }
}

#[cfg(test)]
#[path = "project_layout_tests.rs"]
mod tests;
