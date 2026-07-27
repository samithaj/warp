//! Shared agent-aware tab-title resolution.
//!
//! The vertical tabs list has always named a tab after the agent session
//! running in it (a Claude Code `/rename`, or the Oz conversation title), while
//! the horizontal tab bar only ever saw `PaneGroup::display_title` — the
//! terminal/shell title — so a renamed session showed up in one surface and not
//! the other. This module is the single place that answers "what is the agent
//! in this tab called", so both surfaces can agree.

use warp_core::features::FeatureFlag;
use warpui::{AppContext, SingletonEntity as _};

use crate::pane_group::PaneGroup;
use crate::terminal::cli_agent_sessions::CLIAgentSessionsModel;
use crate::workspace::tab_settings::TabSettings;

/// The agent session name for a tab, if it has one.
///
/// Prefers a plugin-backed CLI agent's own title (what `/rename` updates) over
/// the Oz conversation title. Within each, the
/// `use_latest_user_prompt_as_conversation_title_in_tab_names` setting decides
/// whether the session's title or its latest user prompt wins — the same
/// preference the vertical tabs already honour.
///
/// Returns `None` when the tab hosts no agent, when the CLI agent is not
/// plugin-backed (its title would be stale), or when the user has opted out via
/// `use_agent_session_name_in_tab_titles`.
pub(crate) fn agent_session_title(pane_group: &PaneGroup, app: &AppContext) -> Option<String> {
    if !*TabSettings::as_ref(app).use_agent_session_name_in_tab_titles {
        return None;
    }

    let terminal_view = pane_group.focused_session_view(app)?;
    let terminal_view = terminal_view.as_ref(app);
    let prefer_latest_prompt =
        *TabSettings::as_ref(app).use_latest_user_prompt_as_conversation_title_in_tab_names;

    let cli_agent_session = CLIAgentSessionsModel::as_ref(app).session(terminal_view.id());
    // A CLI agent that isn't plugin-backed never reports title updates, so its
    // context would pin the tab to a stale name. Fall through to the
    // conversation title in that case, matching `terminal_agent_text`.
    let plugin_backed = cli_agent_session.is_some_and(|session| session.listener.is_some());
    if let Some(session) = cli_agent_session.filter(|_| plugin_backed) {
        let context = &session.session_context;
        let cli_title = if prefer_latest_prompt {
            context
                .latest_user_prompt()
                .or_else(|| context.title_like_text())
        } else {
            context.title_like_text()
        };
        if cli_title.is_some() {
            return cli_title;
        }
    }

    if prefer_latest_prompt {
        terminal_view
            .selected_conversation_latest_user_prompt_for_tab_name(app)
            .or_else(|| terminal_view.selected_conversation_display_title(app))
    } else {
        terminal_view
            .selected_conversation_display_title(app)
            .or_else(|| terminal_view.selected_conversation_latest_user_prompt_for_tab_name(app))
    }
}

/// The name to show on a tab: an explicit rename wins, then the agent session
/// name, then the terminal/shell title.
///
/// Gated on `FeatureFlag::Projects` for now because the horizontal tab bar is
/// where the Projects × Tasks layout renders tasks, and naming a task after its
/// agent is only meaningful there.
pub(crate) fn tab_title(pane_group: &PaneGroup, app: &AppContext) -> String {
    if let Some(custom_title) = pane_group.custom_title(app) {
        return custom_title;
    }
    if FeatureFlag::Projects.is_enabled()
        && let Some(agent_title) = agent_session_title(pane_group, app)
    {
        return agent_title;
    }
    pane_group.display_title(app)
}
