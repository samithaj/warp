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
use crate::terminal::{CLIAgent, TerminalView};
use crate::workspace::tab_settings::{TabLineCount, TabPrimaryInfo, TabSecondaryInfo, TabSettings};

/// The agent session name for a tab, if it has one.
///
/// Prefers a plugin-backed CLI agent's own title (what `/rename` updates) over
/// the Oz conversation title. Within each, the
/// `use_latest_user_prompt_as_conversation_title_in_tab_names` setting decides
/// whether the session's title or its latest user prompt wins — the same
/// preference the vertical tabs already honour.
///
/// Returns `None` when the tab hosts no agent, or when the CLI agent is not
/// plugin-backed (its title would be stale), letting the caller fall back to
/// the terminal title.
pub(crate) fn agent_session_title(pane_group: &PaneGroup, app: &AppContext) -> Option<String> {
    let terminal_view = pane_group.focused_session_view(app)?;
    let terminal_view = terminal_view.as_ref(app);
    // Resolved by exactly the same helpers the vertical tabs use, so the two
    // surfaces cannot drift apart on what an agent is called.
    let agent_text = terminal_agent_text(terminal_view, app);
    let (conversation_title, cli_agent_title) =
        preferred_agent_tab_titles(&agent_text, agent_tab_text_preference(app));
    cli_agent_title.or(conversation_title)
}

/// The name to show on a tab: an explicit rename wins, then whatever
/// `TabPrimaryInfo` selects, then the terminal/shell title.
///
/// Gated on `FeatureFlag::Projects` for now because the horizontal tab bar is
/// where the Projects × Tasks layout renders tasks, and naming a task after its
/// agent is only meaningful there.
pub(crate) fn tab_title(pane_group: &PaneGroup, app: &AppContext) -> String {
    if let Some(custom_title) = pane_group.custom_title(app) {
        return custom_title;
    }
    if FeatureFlag::Projects.is_enabled() {
        let primary = TabSettings::as_ref(app).tab_primary_info;
        if let Some(text) = tab_info_text(pane_group, primary.into(), app) {
            return text;
        }
    }
    pane_group.display_title(app)
}

/// The smaller second line of a two-line tab, or `None` when the tab is
/// single-line or there is nothing useful to show.
///
/// The secondary choice is resolved against the primary first, so a tab never
/// shows the same information twice.
pub(crate) fn tab_secondary_line(pane_group: &PaneGroup, app: &AppContext) -> Option<String> {
    let settings = TabSettings::as_ref(app);
    if !FeatureFlag::Projects.is_enabled()
        || !matches!(settings.tab_line_count, TabLineCount::TwoLine)
    {
        return None;
    }
    let secondary = settings
        .tab_secondary_info
        .resolved_for(settings.tab_primary_info);
    tab_info_text(pane_group, secondary.into(), app)
}

/// The distinct kinds of text a tab line can show. Both lines resolve through
/// here so the two settings share one implementation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TabInfoKind {
    AgentSession,
    Command,
    WorkingDirectory,
    Branch,
}

impl From<TabPrimaryInfo> for TabInfoKind {
    fn from(value: TabPrimaryInfo) -> Self {
        match value {
            TabPrimaryInfo::AgentSession => Self::AgentSession,
            TabPrimaryInfo::Command => Self::Command,
            TabPrimaryInfo::WorkingDirectory => Self::WorkingDirectory,
            TabPrimaryInfo::Branch => Self::Branch,
        }
    }
}

impl From<TabSecondaryInfo> for TabInfoKind {
    fn from(value: TabSecondaryInfo) -> Self {
        match value {
            TabSecondaryInfo::AgentSession => Self::AgentSession,
            TabSecondaryInfo::Command => Self::Command,
            TabSecondaryInfo::WorkingDirectory => Self::WorkingDirectory,
            TabSecondaryInfo::Branch => Self::Branch,
        }
    }
}

/// Resolves one kind of tab text for the tab's focused session. Returns `None`
/// when that information isn't available (no agent, no repo, no command yet),
/// letting the caller fall back.
fn tab_info_text(pane_group: &PaneGroup, kind: TabInfoKind, app: &AppContext) -> Option<String> {
    if matches!(kind, TabInfoKind::AgentSession) {
        return agent_session_title(pane_group, app);
    }
    let terminal_view = pane_group.focused_session_view(app)?;
    let terminal_view = terminal_view.as_ref(app);
    let text = match kind {
        // Handled above; listed for exhaustiveness.
        TabInfoKind::AgentSession => None,
        TabInfoKind::Command => terminal_view.last_completed_command_text(),
        TabInfoKind::WorkingDirectory => terminal_view.display_working_directory(app),
        TabInfoKind::Branch => terminal_view.current_git_branch(app),
    };
    text.filter(|text| !text.trim().is_empty())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AgentTabTextPreference {
    ConversationTitle,
    LatestUserPrompt,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct TerminalAgentText {
    pub(crate) conversation_display_title: Option<String>,
    pub(crate) conversation_latest_user_prompt: Option<String>,
    pub(crate) cli_agent_title: Option<String>,
    pub(crate) cli_agent_latest_user_prompt: Option<String>,
    pub(crate) is_oz_agent: bool,
    pub(crate) cli_agent: Option<CLIAgent>,
}

pub(crate) fn agent_tab_text_preference(app: &AppContext) -> AgentTabTextPreference {
    if *TabSettings::as_ref(app).use_latest_user_prompt_as_conversation_title_in_tab_names {
        AgentTabTextPreference::LatestUserPrompt
    } else {
        AgentTabTextPreference::ConversationTitle
    }
}

pub(crate) fn preferred_agent_tab_titles(
    agent_text: &TerminalAgentText,
    preference: AgentTabTextPreference,
) -> (Option<String>, Option<String>) {
    let conversation_title = match preference {
        AgentTabTextPreference::ConversationTitle => agent_text
            .conversation_display_title
            .clone()
            .or_else(|| agent_text.conversation_latest_user_prompt.clone()),
        AgentTabTextPreference::LatestUserPrompt => agent_text
            .conversation_latest_user_prompt
            .clone()
            .or_else(|| agent_text.conversation_display_title.clone()),
    };
    let cli_agent_title = match preference {
        AgentTabTextPreference::ConversationTitle => agent_text.cli_agent_title.clone(),
        AgentTabTextPreference::LatestUserPrompt => agent_text
            .cli_agent_latest_user_prompt
            .clone()
            .or_else(|| agent_text.cli_agent_title.clone()),
    };

    (conversation_title, cli_agent_title)
}

pub(crate) fn terminal_agent_text(
    terminal_view: &TerminalView,
    app: &AppContext,
) -> TerminalAgentText {
    let cli_agent_session = CLIAgentSessionsModel::as_ref(app).session(terminal_view.id());
    let is_plugin_backed = cli_agent_session.is_some_and(|session| session.listener.is_some());
    let is_ambient_agent = terminal_view.is_ambient_agent_session(app);

    let mut agent_text = TerminalAgentText {
        is_oz_agent: is_ambient_agent,
        cli_agent: cli_agent_session.map(|session| session.agent),
        ..Default::default()
    };

    if cli_agent_session.is_some() && !is_plugin_backed {
        return agent_text;
    }

    agent_text.conversation_display_title = terminal_view.selected_conversation_display_title(app);
    agent_text.conversation_latest_user_prompt =
        terminal_view.selected_conversation_latest_user_prompt_for_tab_name(app);
    agent_text.is_oz_agent =
        agent_text.conversation_display_title.is_some() || agent_text.is_oz_agent;

    if let Some(session) = cli_agent_session {
        agent_text.cli_agent_title = session.session_context.title_like_text();
        agent_text.cli_agent_latest_user_prompt = session.session_context.latest_user_prompt();
    }

    agent_text
}

#[cfg(test)]
#[path = "tab_title_tests.rs"]
mod tests;
