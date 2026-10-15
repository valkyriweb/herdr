use std::collections::HashMap;

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
    text::Line,
    widgets::{Paragraph, Widget},
};

use super::*;

struct AgentRow {
    pane_id: String,
    status: crate::api::schema::AgentStatus,
    focused: bool,
    rows: Vec<Vec<crate::ui::ResolvedToken>>,
}

/// Identity of one agent group. `None` is the final group of agents without the group token.
pub(super) type AgentGroupKey = Option<String>;

/// Heading shown for the group of agents that lack the group token.
const OTHER_GROUP_LABEL: &str = "Other";

/// Fold state of one grouped view, owned by the client for the lifetime of one endpoint boot.
pub(super) type AgentGroupFolds = HashMap<(String, String), HashSet<AgentGroupKey>>;

/// How the agent panel groups the active view, when the endpoint named a group token.
#[derive(Clone, Copy)]
pub(super) struct AgentGroupRender<'a> {
    pub(super) token: &'a str,
    pub(super) folded: Option<&'a HashSet<AgentGroupKey>>,
}

impl AgentGroupRender<'_> {
    fn is_folded(&self, key: &AgentGroupKey) -> bool {
        self.folded.is_some_and(|folded| folded.contains(key))
    }
}

/// One group of agents in display order.
pub(super) struct AgentGroup {
    pub(super) key: AgentGroupKey,
    pub(super) pane_ids: Vec<String>,
    attention: usize,
    focused: bool,
}

enum AgentItem {
    Heading {
        key: AgentGroupKey,
        label: String,
        agents: usize,
        attention: usize,
        folded: bool,
        focused: bool,
    },
    Agent(AgentRow),
}

impl AgentItem {
    fn height(&self) -> u16 {
        match self {
            AgentItem::Heading { .. } => 1,
            AgentItem::Agent(row) => row.rows.len().max(1).min(u16::MAX as usize) as u16,
        }
    }
}

/// The attention notion used everywhere else: blocked, or finished and not yet seen.
fn needs_attention(status: crate::api::schema::AgentStatus) -> bool {
    use crate::api::schema::AgentStatus;
    matches!(status, AgentStatus::Blocked | AgentStatus::Done)
}

fn group_value<'a>(agent: &'a crate::protocol::ClientShellAgent, token: &str) -> Option<&'a str> {
    agent
        .tokens
        .iter()
        .find(|(name, _)| name == token)
        .map(|(_, value)| value.as_str())
        .filter(|value| !value.is_empty())
}

/// Splits `ordered` into groups by `token`. Groups keep the order in which their first member
/// appears, members keep their order, and agents without the token form the last group. The
/// endpoint already orders `ordered` this way, so grouping here never reorders a conforming list.
pub(super) fn agent_groups(
    snapshot: &ClientShellSnapshot,
    ordered: &[String],
    token: &str,
) -> Vec<AgentGroup> {
    let agents = snapshot
        .agents
        .iter()
        .map(|agent| (agent.pane_id.as_str(), agent))
        .collect::<HashMap<_, _>>();
    let mut groups: Vec<AgentGroup> = Vec::new();
    let mut index_by_value: HashMap<&str, usize> = HashMap::new();
    let mut other: Option<AgentGroup> = None;
    for pane_id in ordered {
        let Some(agent) = agents.get(pane_id.as_str()) else {
            continue;
        };
        let value = group_value(agent, token);
        let group = match value {
            Some(value) => {
                let next = groups.len();
                let index = *index_by_value.entry(value).or_insert(next);
                if index == next {
                    groups.push(AgentGroup {
                        key: Some(value.to_owned()),
                        pane_ids: Vec::new(),
                        attention: 0,
                        focused: false,
                    });
                }
                &mut groups[index]
            }
            None => other.get_or_insert_with(|| AgentGroup {
                key: None,
                pane_ids: Vec::new(),
                attention: 0,
                focused: false,
            }),
        };
        group.pane_ids.push(pane_id.clone());
        group.attention += usize::from(needs_attention(agent.agent_status));
        group.focused |= agent.focused;
    }
    groups.extend(other);
    groups
}

/// Index of `pane_id` among the panel's items, treating its own group as expanded. Headings count
/// as items; agents in other folded groups do not.
pub(super) fn agent_item_index(
    groups: &[AgentGroup],
    group: AgentGroupRender<'_>,
    pane_id: &str,
) -> Option<usize> {
    let mut index = 0;
    for candidate in groups {
        index += 1;
        if let Some(position) = candidate.pane_ids.iter().position(|id| id == pane_id) {
            return Some(index + position);
        }
        if !group.is_folded(&candidate.key) {
            index += candidate.pane_ids.len();
        }
    }
    None
}

/// The grouping the endpoint last announced, if it describes the current snapshot's active view.
pub(super) fn active_group_view<'a>(
    snapshot: &ClientShellSnapshot,
    view: Option<&'a crate::protocol::endpoint::EndpointAgentView>,
) -> Option<&'a crate::protocol::endpoint::EndpointAgentView> {
    view.filter(|view| {
        view.boot_id == snapshot.boot_id
            && snapshot.agent_view_label.is_some()
            && view.group_by_token.is_some()
    })
}

pub(super) fn fold_key(view: &crate::protocol::endpoint::EndpointAgentView) -> (String, String) {
    (
        view.source.clone().unwrap_or_default(),
        view.group_by_token.clone().unwrap_or_default(),
    )
}

impl ClientShellState {
    /// Records grouping metadata from the endpoint. Returns whether the presentation changed.
    /// Metadata may arrive before the snapshot of its boot, so it is kept until a snapshot of
    /// another boot proves it stale.
    pub(crate) fn set_agent_view(
        &mut self,
        view: crate::protocol::endpoint::EndpointAgentView,
    ) -> bool {
        if self.agent_view.as_ref() == Some(&view) {
            return false;
        }
        self.agent_view = Some(view);
        true
    }

    /// Folds or unfolds one group of the active grouped view.
    pub(super) fn toggle_agent_group(&mut self, key: &AgentGroupKey) {
        let Some(snapshot) = self.snapshot.as_deref() else {
            return;
        };
        let Some(view) = active_group_view(snapshot, self.agent_view.as_ref()) else {
            return;
        };
        let folded = self.agent_group_folds.entry(fold_key(view)).or_default();
        if !folded.remove(key) {
            folded.insert(key.clone());
        }
    }

    /// Unfolds the group holding `pane_id` and returns its item index, so a keyboard focus jump
    /// never lands on a hidden row.
    pub(super) fn reveal_agent_in_group(&mut self, pane_id: &str) -> Option<usize> {
        let snapshot = self.snapshot.as_deref()?;
        let view = active_group_view(snapshot, self.agent_view.as_ref())?;
        let token = view.group_by_token.as_deref()?;
        let ordered = ordered_agent_pane_ids(snapshot, self.config.agent_panel_sort);
        let groups = agent_groups(snapshot, &ordered, token);
        let key = groups
            .iter()
            .find(|group| group.pane_ids.iter().any(|id| id == pane_id))?
            .key
            .clone();
        let fold_key = fold_key(view);
        if let Some(folded) = self.agent_group_folds.get_mut(&fold_key) {
            folded.remove(&key);
        }
        let render = AgentGroupRender {
            token,
            folded: self.agent_group_folds.get(&fold_key),
        };
        agent_item_index(&groups, render, pane_id)
    }
}

pub(super) fn ordered_agent_pane_ids(
    snapshot: &ClientShellSnapshot,
    sort: crate::config::AgentPanelSortConfig,
) -> Vec<String> {
    if snapshot.agent_view_label.is_some() {
        return snapshot
            .agent_order
            .iter()
            .filter(|pane_id| {
                snapshot
                    .agents
                    .iter()
                    .any(|agent| agent.pane_id == pane_id.as_str())
            })
            .cloned()
            .collect();
    }
    let mut agents = snapshot.agents.iter().collect::<Vec<_>>();
    if sort == crate::config::AgentPanelSortConfig::Priority {
        agents.sort_by_key(|agent| {
            (
                std::cmp::Reverse(status_priority(agent.agent_status)),
                std::cmp::Reverse(agent.state_change_seq),
            )
        });
    }
    agents
        .into_iter()
        .map(|agent| agent.pane_id.clone())
        .collect()
}

pub(super) fn render_agent_panel(
    buffer: &mut Buffer,
    area: Rect,
    snapshot: &ClientShellSnapshot,
    config: &ClientShellConfig,
    agent_scroll: &mut usize,
    group: Option<AgentGroupRender<'_>>,
    hits: &mut ShellHitMap,
) {
    if area.height == 0 {
        return;
    }
    put_text(
        buffer,
        area.x,
        area.y,
        area.width,
        &"─".repeat(area.width as usize),
        Style::default().fg(config.palette.surface_dim),
    );
    if area.height < 2 {
        return;
    }
    put_text(
        buffer,
        area.x,
        area.y + 1,
        area.width,
        " agents",
        Style::default()
            .fg(config.palette.overlay0)
            .add_modifier(Modifier::BOLD),
    );
    let sort_label =
        snapshot
            .agent_view_label
            .as_deref()
            .unwrap_or(match config.agent_panel_sort {
                crate::config::AgentPanelSortConfig::Spaces => "spaces",
                crate::config::AgentPanelSortConfig::Priority => "priority",
            });
    let sort_width = display_width(sort_label).min(area.width as usize) as u16;
    let sort_rect = Rect::new(
        area.right().saturating_sub(sort_width),
        area.y + 1,
        sort_width,
        1,
    );
    hits.agent_sort_toggle = if config.mouse_capture {
        sort_rect
    } else {
        Rect::default()
    };
    put_text(
        buffer,
        sort_rect.x,
        sort_rect.y,
        sort_rect.width,
        sort_label,
        Style::default()
            .fg(if snapshot.agent_view_label.is_some() {
                config.palette.accent
            } else {
                config.palette.overlay0
            })
            .add_modifier(Modifier::BOLD),
    );

    let rows = agent_items(snapshot, config, group);
    let body = Rect::new(
        area.x,
        area.y.saturating_add(3),
        area.width,
        area.height.saturating_sub(3),
    );
    hits.agent_body = body;
    if body.is_empty() || rows.is_empty() {
        *agent_scroll = 0;
        if !body.is_empty() && snapshot.agent_view_label.is_some() {
            put_text(
                buffer,
                body.x,
                body.y,
                body.width,
                " no matching agents",
                Style::default()
                    .fg(config.palette.overlay0)
                    .add_modifier(Modifier::DIM),
            );
        }
        return;
    }

    let row_heights = rows.iter().map(AgentItem::height).collect::<Vec<_>>();
    // Headings sit directly on their first row, so only agent rows are separated by the gap.
    let gap_after = |index: usize| {
        if index + 1 < rows.len() && matches!(rows[index], AgentItem::Agent(_)) {
            config.agents.row_gap
        } else {
            0
        }
    };
    let gaps = (0..rows.len()).map(gap_after).collect::<Vec<_>>();
    let metrics =
        super::scroll::list_scroll_metrics(&row_heights, &gaps, body.height, *agent_scroll);
    hits.agent_max_scroll = metrics.max_offset_from_bottom;
    hits.agent_scroll_metrics = Some(metrics);
    *agent_scroll = metrics
        .max_offset_from_bottom
        .saturating_sub(metrics.offset_from_bottom);
    let show_scrollbar = metrics.max_offset_from_bottom > 0 && body.width > 1;
    let content_width = body.width.saturating_sub(u16::from(show_scrollbar));
    let mut y = body.y;
    for (index, item) in rows.iter().enumerate().skip(*agent_scroll) {
        let height = item.height().min(body.height);
        if y.saturating_add(height) > body.bottom() {
            break;
        }
        let rect = Rect::new(body.x, y, content_width, height);
        match item {
            AgentItem::Heading {
                key,
                label,
                agents,
                attention,
                folded,
                focused,
            } => {
                hits.agent_group_headings.push((rect, key.clone()));
                render_group_heading(
                    buffer, rect, label, *agents, *attention, *folded, *focused, config,
                );
            }
            AgentItem::Agent(row) => {
                hits.agents.push((rect, row.pane_id.clone()));
                render_agent_row(buffer, rect, row, config);
            }
        }
        y = y.saturating_add(height).saturating_add(gap_after(index));
    }

    if show_scrollbar {
        let track = Rect::new(body.right().saturating_sub(1), body.y, 1, body.height);
        hits.agent_scrollbar = track;
        super::scroll::render_list_scrollbar(buffer, track, metrics, &config.palette);
    }
}

fn agent_items(
    snapshot: &ClientShellSnapshot,
    config: &ClientShellConfig,
    group: Option<AgentGroupRender<'_>>,
) -> Vec<AgentItem> {
    let rows = agent_rows(snapshot, config);
    let Some(group) = group else {
        return rows.into_iter().map(AgentItem::Agent).collect();
    };
    let ordered = rows
        .iter()
        .map(|row| row.pane_id.clone())
        .collect::<Vec<_>>();
    let groups = agent_groups(snapshot, &ordered, group.token);
    let mut rows = rows
        .into_iter()
        .map(|row| (row.pane_id.clone(), row))
        .collect::<HashMap<_, _>>();
    let mut items = Vec::with_capacity(ordered.len() + groups.len());
    for agent_group in groups {
        let folded = group.is_folded(&agent_group.key);
        items.push(AgentItem::Heading {
            label: agent_group
                .key
                .clone()
                .unwrap_or_else(|| OTHER_GROUP_LABEL.to_owned()),
            key: agent_group.key,
            agents: agent_group.pane_ids.len(),
            attention: agent_group.attention,
            folded,
            focused: agent_group.focused,
        });
        if !folded {
            items.extend(
                agent_group
                    .pane_ids
                    .iter()
                    .filter_map(|pane_id| rows.remove(pane_id))
                    .map(AgentItem::Agent),
            );
        }
    }
    items
}

fn agent_rows(snapshot: &ClientShellSnapshot, config: &ClientShellConfig) -> Vec<AgentRow> {
    ordered_agent_pane_ids(snapshot, config.agent_panel_sort)
        .into_iter()
        .filter_map(|pane_id| {
            let agent = snapshot
                .agents
                .iter()
                .find(|agent| agent.pane_id == pane_id)?;
            let workspace = snapshot
                .workspaces
                .iter()
                .find(|workspace| workspace.workspace_id == agent.workspace_id)?;
            let tab = snapshot.tabs.iter().find(|tab| tab.tab_id == agent.tab_id);
            let pane = snapshot
                .panes
                .iter()
                .find(|pane| pane.pane_id == agent.pane_id);
            let tab_count = snapshot
                .tabs
                .iter()
                .filter(|candidate| candidate.workspace_id == agent.workspace_id)
                .count();
            let tab_label = tab
                .filter(|tab| tab_count > 1 || tab.custom_label)
                .map(|tab| tab.label.as_str());
            let agent_label = agent
                .display_agent
                .as_deref()
                .or(agent.name.as_deref())
                .or(agent.agent.as_deref())
                .or(agent.title.as_deref());
            let labels = agent
                .state_labels
                .iter()
                .cloned()
                .collect::<HashMap<_, _>>();
            let tokens = agent.tokens.iter().cloned().collect::<HashMap<_, _>>();
            let state_text = labels
                .get(status_text(agent.agent_status))
                .map(String::as_str)
                .unwrap_or_else(|| sidebar_status_text(agent.agent_status));
            let canonical_agent = agent
                .agent
                .as_deref()
                .and_then(crate::detect::parse_agent_label);
            let rows = crate::ui::sidebar_agent_rows(
                &config.agents,
                crate::ui::AgentTokenContext {
                    workspace: &workspace.label,
                    tab: tab_label,
                    pane: agent
                        .title
                        .as_deref()
                        .or_else(|| pane.and_then(|pane| pane.label.as_deref())),
                    agent_label,
                    terminal_title: agent.terminal_title.as_deref(),
                    terminal_title_stripped: agent.terminal_title_stripped.as_deref(),
                    canonical_agent,
                    tokens: &tokens,
                },
                state_text,
            );
            Some(AgentRow {
                pane_id: agent.pane_id.clone(),
                status: agent.agent_status,
                focused: agent.focused,
                rows,
            })
        })
        .collect()
}

#[allow(clippy::too_many_arguments)] // Why: one heading is drawn from its already-derived facts.
fn render_group_heading(
    buffer: &mut Buffer,
    rect: Rect,
    label: &str,
    agents: usize,
    attention: usize,
    folded: bool,
    focused: bool,
    config: &ClientShellConfig,
) {
    let palette = &config.palette;
    let row_style = if focused && folded {
        Style::default().bg(palette.active_row_bg)
    } else {
        Style::default()
    };
    buffer.set_style(rect, row_style);
    let name_style = row_style.fg(palette.subtext0).add_modifier(Modifier::BOLD);
    let count_style = row_style.fg(palette.overlay0);
    let attention_style = row_style.fg(palette.red).add_modifier(Modifier::BOLD);
    let marker = if folded { "▸" } else { "▾" };
    let mut counts = vec![ratatui::text::Span::styled(agents.to_string(), count_style)];
    if attention > 0 {
        counts.push(ratatui::text::Span::styled(
            format!(" !{attention}"),
            attention_style,
        ));
    }
    let counts_width = counts
        .iter()
        .map(|span| display_width(&span.content))
        .sum::<usize>();
    let width = rect.width as usize;
    // The count block wins the right edge; the name is clipped to what remains.
    let counts_width = counts_width.min(width.saturating_sub(2));
    let name_width = width.saturating_sub(counts_width + 4);
    let mut name = String::new();
    let mut used = 0;
    for ch in label.chars() {
        let ch_width = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
        if used + ch_width > name_width {
            break;
        }
        used += ch_width;
        name.push(ch);
    }
    let padding = width.saturating_sub(2 + used + counts_width + 1);
    let mut spans = vec![
        ratatui::text::Span::styled(format!(" {marker}"), count_style),
        ratatui::text::Span::styled(format!(" {name}"), name_style),
        ratatui::text::Span::styled(" ".repeat(padding), row_style),
    ];
    spans.extend(counts);
    Paragraph::new(Line::from(spans))
        .style(row_style)
        .render(rect, buffer);
}

fn render_agent_row(buffer: &mut Buffer, rect: Rect, row: &AgentRow, config: &ClientShellConfig) {
    let palette = &config.palette;
    let row_style = if row.focused {
        Style::default().bg(palette.active_row_bg)
    } else {
        Style::default()
    };
    let name_style = if row.focused {
        Style::default()
            .fg(palette.text)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
            .fg(palette.subtext0)
            .add_modifier(Modifier::BOLD)
    };
    let status_style = Style::default()
        .fg(status_color(row.status, palette))
        .add_modifier(if row.focused {
            Modifier::empty()
        } else {
            Modifier::DIM
        });
    let secondary = Style::default()
        .fg(palette.overlay0)
        .add_modifier(Modifier::DIM);
    let icon = (
        status_icon(row.status, config.status_indicators),
        Style::default().fg(status_color(row.status, palette)),
    );
    let rows = if row.rows.is_empty() {
        vec![vec![crate::ui::ResolvedToken {
            kind: crate::ui::ResolvedTokenKind::StateIcon,
            style: Default::default(),
        }]]
    } else {
        row.rows.clone()
    };
    for (index, tokens) in rows.iter().take(rect.height as usize).enumerate() {
        let indent = if index == 0 { 1 } else { 3 };
        let mut spans = vec![ratatui::text::Span::raw(" ".repeat(indent))];
        spans.extend(crate::ui::resolved_token_spans(
            tokens,
            icon,
            status_style,
            name_style,
            secondary,
            secondary,
            palette,
            rect.width.saturating_sub(indent as u16) as usize,
        ));
        Paragraph::new(Line::from(spans)).style(row_style).render(
            Rect::new(rect.x, rect.y + index as u16, rect.width, 1),
            buffer,
        );
    }
}

fn put_text(buffer: &mut Buffer, x: u16, y: u16, width: u16, text: &str, style: Style) {
    for (offset, character) in text.chars().take(width as usize).enumerate() {
        if let Some(cell) = buffer.cell_mut((x + offset as u16, y)) {
            cell.set_char(character).set_style(style);
        }
    }
}

fn display_width(text: &str) -> usize {
    unicode_width::UnicodeWidthStr::width(text)
}

fn sidebar_status_text(status: crate::api::schema::AgentStatus) -> &'static str {
    use crate::api::schema::AgentStatus;
    match status {
        AgentStatus::Blocked => "blocked",
        AgentStatus::Done => "done",
        AgentStatus::Working => "working",
        AgentStatus::Idle | AgentStatus::Unknown => "idle",
    }
}
