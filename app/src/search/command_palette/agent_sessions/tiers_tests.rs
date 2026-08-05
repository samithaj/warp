use ordered_float::OrderedFloat;
use warpui::elements::Empty;
use warpui::{AppContext, Element};

use super::*;
use crate::appearance::Appearance;
use crate::search::SearchItem;
use crate::search::command_palette::mixer::CommandPaletteItemAction;
use crate::search::command_palette::separator_search_item::SeparatorSearchItem;
use crate::search::data_source::QueryResult;
use crate::search::result_renderer::ItemHighlightState;

/// A stand-in row for whichever tier a test needs one in — including the
/// content tier, which Phase 1 has no producer for yet. Ordering has to be
/// assertable before the rows exist, because the failure it guards against
/// (a tier scheme that renders upside down) is invisible until they do.
#[derive(Debug)]
struct TieredRow {
    label: String,
    priority_tier: u8,
    score: f64,
}

impl SearchItem for TieredRow {
    type Action = CommandPaletteItemAction;

    fn render_icon(
        &self,
        _highlight_state: ItemHighlightState,
        _appearance: &Appearance,
    ) -> Box<dyn Element> {
        Empty::new().finish()
    }

    fn render_item(
        &self,
        _highlight_state: ItemHighlightState,
        _app: &AppContext,
    ) -> Box<dyn Element> {
        Empty::new().finish()
    }

    fn priority_tier(&self) -> u8 {
        self.priority_tier
    }

    fn score(&self) -> OrderedFloat<f64> {
        OrderedFloat(self.score)
    }

    fn accept_result(&self) -> Self::Action {
        CommandPaletteItemAction::NoOp
    }

    fn execute_result(&self) -> Self::Action {
        self.accept_result()
    }

    fn accessibility_label(&self) -> String {
        self.label.clone()
    }
}

fn row(label: &str, priority_tier: u8, score: f64) -> QueryResult<CommandPaletteItemAction> {
    QueryResult::from(TieredRow {
        label: label.to_owned(),
        priority_tier,
        score,
    })
}

fn separator(title: &str, priority_tier: u8) -> QueryResult<CommandPaletteItemAction> {
    QueryResult::from(SeparatorSearchItem::new(title.to_owned()).with_priority_tier(priority_tier))
}

/// Reproduces what the palette does to a result set: the mixer sorts ascending
/// on `(priority_tier, score)` and a `TopDown` search bar then reverses, so the
/// label order this returns is the on-screen order, top to bottom.
fn rendered_labels(mut results: Vec<QueryResult<CommandPaletteItemAction>>) -> Vec<String> {
    results.sort_by_key(|result| (result.priority_tier(), result.score()));
    results
        .into_iter()
        .rev()
        .map(|result| result.accessibility_label())
        .collect()
}

#[test]
fn sections_render_names_above_content_with_each_header_above_its_rows() {
    // Deliberately shuffled, and with the content rows scoring *higher* than
    // the name rows: tiers, not scores, decide which section comes first.
    let labels = rendered_labels(vec![
        row("content row b", CONTENT_ROW_TIER, 900.),
        separator(NAME_SEPARATOR_TITLE, NAME_SEPARATOR_TIER),
        row("name row b", NAME_ROW_TIER, 5.),
        row("content row a", CONTENT_ROW_TIER, 990.),
        separator(CONTENT_SEPARATOR_TITLE, CONTENT_SEPARATOR_TIER),
        row("name row a", NAME_ROW_TIER, 50.),
    ]);

    assert_eq!(
        labels,
        vec![
            format!("Section: {NAME_SEPARATOR_TITLE}"),
            "name row a".to_owned(),
            "name row b".to_owned(),
            format!("Section: {CONTENT_SEPARATOR_TITLE}"),
            "content row a".to_owned(),
            "content row b".to_owned(),
        ],
        "higher tier must render higher: swapping any two tier constants \
         inverts this silently, with no compile error"
    );
}

#[test]
fn the_first_selectable_row_is_at_rendered_index_one() {
    // The whole justification for `set_initial_selection_offset(1)`:
    // `SelectionUpdate::Top` does not skip non-interactable items the way
    // Up/Down do, so index 0 lands on the header and Enter silently does
    // nothing — while index 1 is the first real row. Both halves have to hold,
    // so both are asserted here rather than only described in a comment.
    let mut results = vec![
        row("name row", NAME_ROW_TIER, 1.),
        separator(NAME_SEPARATOR_TITLE, NAME_SEPARATOR_TIER),
    ];
    results.sort_by_key(|result| (result.priority_tier(), result.score()));
    let rendered: Vec<_> = results.into_iter().rev().collect();

    assert!(rendered.len() >= 2);
    assert!(
        rendered[0].is_static_separator(),
        "index 0 is the section header, which cannot be accepted"
    );
    assert!(
        !rendered[1].is_static_separator(),
        "index 1 — what the offset selects — must be a real row"
    );
    assert_eq!(rendered[1].accessibility_label(), "name row");
}
