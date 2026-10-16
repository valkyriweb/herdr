use super::*;
use std::hint::black_box;
use std::time::{Duration, Instant};

const TOKEN: &str = "hp_project";
const SOURCE: &str = "plugin:example.projects";

fn agent(pane_id: &str, project: Option<&str>, status: AgentStatus) -> ClientShellAgent {
    ClientShellAgent {
        pane_id: pane_id.into(),
        workspace_id: "ws_1".into(),
        tab_id: "tab_1".into(),
        name: Some(pane_id.into()),
        display_agent: None,
        agent: Some("pi".into()),
        title: None,
        terminal_title: None,
        terminal_title_stripped: None,
        agent_status: status,
        state_change_seq: 1,
        state_labels: Vec::new(),
        tokens: project
            .map(|value| vec![(TOKEN.to_owned(), value.to_owned())])
            .unwrap_or_default(),
        focused: false,
    }
}

/// A snapshot whose active view orders `agents` exactly as given, as the server would.
fn grouped_snapshot(agents: Vec<ClientShellAgent>) -> ClientShellSnapshot {
    let mut projected = snapshot();
    let template = projected.panes[0].clone();
    for agent in &agents {
        if agent.pane_id != template.pane_id {
            let mut pane = template.clone();
            pane.pane_id = agent.pane_id.clone();
            pane.focused = false;
            projected.panes.push(pane);
        }
    }
    projected.agent_view_label = Some("projects".into());
    projected.agent_order = agents.iter().map(|agent| agent.pane_id.clone()).collect();
    projected.agents = agents;
    projected
}

/// The active endpoint's `endpoint.agent-view.v1` view value; `token` is its optional `group_by`.
fn view(source: &str, token: Option<&str>) -> crate::api::schema::AgentViewSetParams {
    crate::api::schema::AgentViewSetParams {
        source: source.to_owned(),
        label: Some("projects".into()),
        filter: None,
        sort: Vec::new(),
        group_by: token.map(|token| crate::api::schema::AgentViewGroupBy {
            token: token.to_owned(),
        }),
    }
}

/// Announces the view for the snapshot the state currently holds, as the server's
/// revision-bound projection does.
fn announce(state: &mut ClientShellState, source: &str, token: Option<&str>) {
    state.set_test_endpoint_agent_view(&ClientEndpointId::Local, Some(view(source, token)));
}

fn shell_config() -> ClientShellConfig {
    let mut config = Config::default();
    config.ui.sidebar.agents.rows = vec![vec![crate::config::AgentSidebarToken::Agent]];
    config.ui.sidebar.agents.row_gap = 0;
    ClientShellConfig::from_config(&config)
}

fn grouped_state(
    agents: Vec<ClientShellAgent>,
    token: Option<&str>,
) -> (ClientShellState, crate::client::frame_output::ComposedFrame) {
    let mut state = ClientShellState::new(shell_config());
    state.set_agent_view_group_by_supported(true);
    state.set_snapshot(Box::new(grouped_snapshot(agents)));
    announce(&mut state, SOURCE, token);
    state.set_pane_surface(surface());
    let frame = state.compose(106, 30).expect("grouped sidebar frame");
    (state, frame)
}

fn frame_rows(frame: &FrameData) -> Vec<String> {
    frame
        .cells
        .chunks(frame.width as usize)
        .map(|row| row.iter().map(|cell| cell.symbol.as_str()).collect())
        .collect()
}

/// Text of the sidebar column only, one entry per frame row.
fn sidebar_rows(state: &ClientShellState, frame: &FrameData) -> Vec<String> {
    let body = state.hits.agent_body;
    frame_rows(frame)
        .into_iter()
        .skip(body.y as usize)
        .take(body.height as usize)
        .map(|row| {
            row.chars()
                .skip(body.x as usize)
                .take(body.width as usize)
                .collect::<String>()
        })
        .collect()
}

fn heading_rows(state: &ClientShellState, frame: &FrameData) -> Vec<String> {
    sidebar_rows(state, frame)
        .into_iter()
        .filter(|row| row.contains('▾') || row.contains('▸'))
        .map(|row| row.split_whitespace().collect::<Vec<_>>().join(" "))
        .collect()
}

fn visible_agents(state: &ClientShellState) -> Vec<&str> {
    state
        .hits
        .agents
        .iter()
        .map(|(_, pane_id)| pane_id.as_str())
        .collect()
}

fn click(state: &mut ClientShellState, rect: Rect) -> ClientShellInput {
    let at = |kind| {
        RawInputEvent::Mouse(MouseEvent {
            kind,
            column: rect.x,
            row: rect.y,
            modifiers: KeyModifiers::empty(),
        })
    };
    state.handle_raw_events(vec![
        at(MouseEventKind::Down(MouseButton::Left)),
        at(MouseEventKind::Up(MouseButton::Left)),
    ])
}

fn focus_target(input: &ClientShellInput) -> &str {
    let [ClientShellAction::Endpoint { request, .. }] = &input.actions[..] else {
        panic!(
            "expected exactly one endpoint action: {:?}",
            input.actions.len()
        );
    };
    let crate::api::schema::Method::PaneFocus(target) = &request.method else {
        panic!("expected pane.focus");
    };
    &target.pane_id
}

fn binding(state: &mut ClientShellState, action: crate::input::KeybindAction) -> ClientShellInput {
    let mut input = ClientShellInput::default();
    state.record_binding(crate::input::KeybindMatch::Action(action), &mut input);
    input
}

fn standard_agents() -> Vec<ClientShellAgent> {
    vec![
        agent("a1", Some("alpha"), AgentStatus::Blocked),
        agent("a2", Some("alpha"), AgentStatus::Done),
        agent("a3", Some("alpha"), AgentStatus::Idle),
        agent("b1", Some("beta"), AgentStatus::Working),
        agent("b2", Some("beta"), AgentStatus::Idle),
        agent("x1", None, AgentStatus::Idle),
        agent("x2", Some(""), AgentStatus::Done),
    ]
}

#[test]
fn grouped_sidebar_renders_headings_counts_attention_and_other_last() {
    let (state, frame) = grouped_state(standard_agents(), Some(TOKEN));
    assert_eq!(
        heading_rows(&state, &frame),
        vec!["▾ alpha 3 !1", "▾ beta 2", "▾ Other 2"]
    );
    // Rows keep server order under their headings; the heading rows are hit targets of their own.
    assert_eq!(
        visible_agents(&state),
        vec!["a1", "a2", "a3", "b1", "b2", "x1", "x2"]
    );
    assert_eq!(
        state
            .hits
            .agent_group_headings
            .iter()
            .map(|(_, key)| key.as_deref())
            .collect::<Vec<_>>(),
        vec![Some("alpha"), Some("beta"), None]
    );
    // Hit rectangles are disjoint and ordered by their on-screen rows.
    let mut rects = state
        .hits
        .agent_group_headings
        .iter()
        .map(|(rect, _)| *rect)
        .chain(state.hits.agents.iter().map(|(rect, _)| *rect))
        .collect::<Vec<_>>();
    rects.sort_by_key(|rect| rect.y);
    assert!(rects.windows(2).all(|pair| pair[0].bottom() <= pair[1].y));
    let alpha_heading = state.hits.agent_group_headings[0].0;
    assert_eq!(state.hits.agents[0].0.y, alpha_heading.y + 1);
    let beta_heading = state.hits.agent_group_headings[1].0;
    assert_eq!(beta_heading.y, state.hits.agents[2].0.y + 1);
}

#[test]
fn grouped_sidebar_attention_excludes_idle_working_and_unknown() {
    let agents = vec![
        agent("a1", Some("alpha"), AgentStatus::Idle),
        agent("a2", Some("alpha"), AgentStatus::Working),
        agent("a3", Some("alpha"), AgentStatus::Unknown),
    ];
    let (state, frame) = grouped_state(agents, Some(TOKEN));
    assert_eq!(heading_rows(&state, &frame), vec!["▾ alpha 3"]);
}

#[test]
fn clicking_a_heading_folds_and_unfolds_without_any_endpoint_action() {
    let (mut state, _) = grouped_state(standard_agents(), Some(TOKEN));
    let alpha = state.hits.agent_group_headings[0].0;
    let input = click(&mut state, alpha);
    assert!(input.actions.is_empty(), "fold is client-only");
    assert!(input.requests.is_empty(), "fold sends no protocol request");
    assert!(input.repaint);

    let frame = state.compose(106, 30).expect("folded frame");
    assert_eq!(
        heading_rows(&state, &frame),
        vec!["▸ alpha 3 !1", "▾ beta 2", "▾ Other 2"]
    );
    assert_eq!(visible_agents(&state), vec!["b1", "b2", "x1", "x2"]);
    assert_eq!(
        state.agent_group_folds[&(SOURCE.to_owned(), TOKEN.to_owned())],
        HashSet::from([Some("alpha".to_owned())])
    );

    // Folding the trailing group by its heading, then unfolding the first again.
    let other = state.hits.agent_group_headings[2].0;
    assert!(click(&mut state, other).actions.is_empty());
    let alpha = state.hits.agent_group_headings[0].0;
    assert!(click(&mut state, alpha).actions.is_empty());
    let frame = state.compose(106, 30).expect("refolded frame");
    assert_eq!(
        heading_rows(&state, &frame),
        vec!["▾ alpha 3 !1", "▾ beta 2", "▸ Other 2"]
    );
    assert_eq!(visible_agents(&state), vec!["a1", "a2", "a3", "b1", "b2"]);
}

#[test]
fn clicking_an_agent_row_under_a_heading_still_focuses_through_the_endpoint() {
    let (mut state, _) = grouped_state(standard_agents(), Some(TOKEN));
    let b1 = state
        .hits
        .agents
        .iter()
        .find(|(_, pane_id)| pane_id == "b1")
        .expect("b1 row")
        .0;
    let input = click(&mut state, b1);
    assert_eq!(focus_target(&input), "b1");
}

#[test]
fn all_groups_folded_lists_only_headings_with_counts() {
    let (mut state, _) = grouped_state(standard_agents(), Some(TOKEN));
    for index in 0..3 {
        let rect = state.hits.agent_group_headings[index].0;
        click(&mut state, rect);
        state.compose(106, 30).expect("recompose");
    }
    let frame = state.compose(106, 30).expect("all folded frame");
    assert_eq!(
        heading_rows(&state, &frame),
        vec!["▸ alpha 3 !1", "▸ beta 2", "▸ Other 2"]
    );
    assert!(state.hits.agents.is_empty());
    assert_eq!(state.hits.agent_group_headings.len(), 3);
    // Every heading stays clickable and re-expands its own group only.
    let beta = state.hits.agent_group_headings[1].0;
    click(&mut state, beta);
    state.compose(106, 30).expect("beta reopened");
    assert_eq!(visible_agents(&state), vec!["b1", "b2"]);
}

#[test]
fn group_by_projection_is_flat_without_the_server_capability() {
    let agents = standard_agents();
    let mut baseline = ClientShellState::new(shell_config());
    baseline.set_snapshot(Box::new(grouped_snapshot(agents.clone())));
    baseline.set_pane_surface(surface());
    let expected = baseline.compose(106, 30).expect("flat baseline");

    let mut state = ClientShellState::new(shell_config());
    state.set_agent_view_group_by_supported(false);
    state.set_snapshot(Box::new(grouped_snapshot(agents)));
    announce(&mut state, SOURCE, Some(TOKEN));
    state.set_pane_surface(surface());
    let frame = state.compose(106, 30).expect("flat unsupported view");
    assert!(state.hits.agent_group_headings.is_empty());
    assert_eq!(frame_rows(&frame), frame_rows(&expected));
}

#[test]
fn flat_views_and_unmatched_metadata_render_exactly_the_old_list() {
    let agents = standard_agents();
    let mut baseline = ClientShellState::new(shell_config());
    baseline.set_snapshot(Box::new(grouped_snapshot(agents.clone())));
    baseline.set_pane_surface(surface());
    let expected = baseline.compose(106, 30).expect("baseline frame");
    assert!(baseline.hits.agent_group_headings.is_empty());
    assert_eq!(visible_agents(&baseline).len(), 7);

    // A view that names no group_by (an upstream server), and a projection for another boot,
    // change nothing.
    for other_boot in [false, true] {
        let mut state = ClientShellState::new(shell_config());
        state.set_snapshot(Box::new(grouped_snapshot(agents.clone())));
        if other_boot {
            state.set_endpoint_agent_view_projection_supported(&ClientEndpointId::Local, true);
            state.set_test_endpoint_agent_view_projection(
                &ClientEndpointId::Local,
                "boot-other",
                1,
                Some(view(SOURCE, Some(TOKEN))),
            );
        } else {
            announce(&mut state, SOURCE, None);
        }
        state.set_pane_surface(surface());
        let frame = state.compose(106, 30).expect("flat frame");
        assert!(state.hits.agent_group_headings.is_empty());
        assert_eq!(frame_rows(&frame), frame_rows(&expected));
    }

    // Grouping metadata without an active agent view (priority/default list) is ignored too.
    let mut state = ClientShellState::new(shell_config());
    let mut unviewed = grouped_snapshot(agents);
    unviewed.agent_view_label = None;
    unviewed.agent_order.clear();
    state.set_snapshot(Box::new(unviewed));
    announce(&mut state, SOURCE, Some(TOKEN));
    state.set_pane_surface(surface());
    state.compose(106, 30).expect("unviewed frame");
    assert!(state.hits.agent_group_headings.is_empty());
}

#[test]
fn keyboard_focus_agent_reveals_a_folded_target_group() {
    let (mut state, _) = grouped_state(standard_agents(), Some(TOKEN));
    for index in [1, 0] {
        let rect = state.hits.agent_group_headings[index].0;
        click(&mut state, rect);
        state.compose(106, 30).expect("recompose");
    }
    assert_eq!(visible_agents(&state), vec!["x1", "x2"]);

    // Agent index 3 is b1, inside the folded beta group.
    let input = binding(&mut state, crate::input::KeybindAction::FocusAgent(3));
    assert_eq!(focus_target(&input), "b1");
    let frame = state.compose(106, 30).expect("revealed frame");
    assert_eq!(
        heading_rows(&state, &frame),
        vec!["▸ alpha 3 !1", "▾ beta 2", "▾ Other 2"]
    );
    assert_eq!(visible_agents(&state), vec!["b1", "b2", "x1", "x2"]);
}

fn many_group_state(groups: usize, rows: u16) -> ClientShellState {
    let agents = (0..groups)
        .flat_map(|group| {
            (0..3).map(move |member| {
                agent(
                    &format!("g{group}m{member}"),
                    Some(&format!("group{group}")),
                    AgentStatus::Idle,
                )
            })
        })
        .collect::<Vec<_>>();
    let mut state = ClientShellState::new(shell_config());
    state.set_agent_view_group_by_supported(true);
    state.set_snapshot(Box::new(grouped_snapshot(agents)));
    announce(&mut state, SOURCE, Some(TOKEN));
    state.set_pane_surface(surface());
    state.compose(106, rows).expect("many group frame");
    state
}

fn fold(state: &mut ClientShellState, groups: &[&str]) {
    state.agent_group_folds.insert(
        (SOURCE.to_owned(), TOKEN.to_owned()),
        groups
            .iter()
            .map(|group| Some((*group).to_owned()))
            .collect(),
    );
}

#[test]
fn keyboard_next_and_previous_reveal_folded_targets_and_scroll_to_them() {
    let mut state = many_group_state(6, 14);
    state.snapshot.as_mut().expect("snapshot").focused_pane_id = Some("g1m2".into());
    // 24 items (6 headings + 18 agents) cannot fit a short sidebar.
    assert!(state.hits.agent_max_scroll > 0);
    fold(&mut state, &["group2"]);
    state.compose(106, 14).expect("group2 folded");
    assert!(!visible_agents(&state).contains(&"g2m0"));

    let input = binding(&mut state, crate::input::KeybindAction::NextAgent);
    assert_eq!(focus_target(&input), "g2m0");
    assert!(state.agent_group_folds[&(SOURCE.to_owned(), TOKEN.to_owned())].is_empty());
    // Items before g2m0: heading0, 3 agents, heading1, 3 agents, heading2.
    assert_eq!(state.agent_scroll, 9);
    state.compose(106, 14).expect("scrolled frame");
    assert!(
        visible_agents(&state).contains(&"g2m0"),
        "revealed target must be on screen: {:?}",
        visible_agents(&state)
    );

    // Previous from the first agent wraps to the last group, which is folded.
    state.snapshot.as_mut().expect("snapshot").focused_pane_id = Some("g0m0".into());
    state.agent_scroll = 0;
    fold(&mut state, &["group5"]);
    state.compose(106, 14).expect("group5 folded");
    let input = binding(&mut state, crate::input::KeybindAction::PreviousAgent);
    assert_eq!(focus_target(&input), "g5m2");
    state.compose(106, 14).expect("wrapped frame");
    assert!(visible_agents(&state).contains(&"g5m2"));
}

#[test]
fn folding_a_group_while_scrolled_to_the_bottom_clamps_without_a_blank_panel() {
    let mut state = many_group_state(6, 14);
    state.agent_scroll = usize::MAX;
    let frame = state.compose(106, 14).expect("bottom frame");
    let bottom_scroll = state.agent_scroll;
    assert!(bottom_scroll > 0);
    assert_eq!(bottom_scroll, state.hits.agent_max_scroll);
    assert!(!visible_agents(&state).is_empty());
    let _ = frame;

    // Fold the first visible heading; every other group stays open below it.
    let heading = state
        .hits
        .agent_group_headings
        .first()
        .map(|(rect, _)| *rect)
        .expect("a heading is visible at the bottom");
    click(&mut state, heading);
    let frame = state.compose(106, 14).expect("folded bottom frame");
    assert!(
        state.agent_scroll <= state.hits.agent_max_scroll,
        "scroll {} must clamp to max {}",
        state.agent_scroll,
        state.hits.agent_max_scroll
    );
    assert!(
        state.agent_scroll < bottom_scroll,
        "fewer items shrink the offset"
    );
    let rows = sidebar_rows(&state, &frame);
    assert!(
        rows.iter().filter(|row| !row.trim().is_empty()).count() >= rows.len().min(8),
        "panel must stay filled: {rows:?}"
    );
    assert!(!state.hits.agents.is_empty() || !state.hits.agent_group_headings.is_empty());

    // Folding every group leaves a short list that cannot scroll at all.
    fold(
        &mut state,
        &["group0", "group1", "group2", "group3", "group4", "group5"],
    );
    let frame = state.compose(106, 14).expect("all folded frame");
    assert!(state.agent_scroll <= state.hits.agent_max_scroll);
    let drawn = heading_rows(&state, &frame).len();
    assert_eq!(drawn, state.hits.agent_group_headings.len());
    assert_eq!(drawn, state.hits.agent_body.height.min(6) as usize);
}

#[test]
fn clearing_the_view_removes_headings_and_restoring_it_keeps_session_folds() {
    let (mut state, _) = grouped_state(standard_agents(), Some(TOKEN));
    let beta = state.hits.agent_group_headings[1].0;
    click(&mut state, beta);
    state.compose(106, 30).expect("beta folded");
    assert_eq!(visible_agents(&state).len(), 5);

    // Clear: no view label and a flat control. Headings and fold effects disappear.
    let mut cleared = snapshot();
    cleared.revision = 2;
    cleared.agents = standard_agents();
    state.set_snapshot(Box::new(cleared));
    announce(&mut state, SOURCE, None);
    state.set_pane_surface({
        let mut surface = surface();
        surface.projection_revision = 2;
        surface
    });
    let frame = state.compose(106, 30).expect("cleared frame");
    assert!(heading_rows(&state, &frame).is_empty());
    assert!(state.hits.agent_group_headings.is_empty());
    assert_eq!(
        visible_agents(&state).len(),
        7,
        "flat list lists every agent"
    );

    // Restore the same source and token: the fold comes back without any server involvement.
    let mut restored = grouped_snapshot(standard_agents());
    restored.revision = 3;
    state.set_snapshot(Box::new(restored));
    announce(&mut state, SOURCE, Some(TOKEN));
    state.set_pane_surface({
        let mut surface = surface();
        surface.projection_revision = 3;
        surface
    });
    let frame = state.compose(106, 30).expect("restored frame");
    assert_eq!(
        heading_rows(&state, &frame),
        vec!["▾ alpha 3 !1", "▸ beta 2", "▾ Other 2"]
    );

    // Another owner starts with every group open; another token groups by that token instead.
    for (offset, (source, token, headings)) in
        [("plugin:other", TOKEN, 3), (SOURCE, "other_token", 1)]
            .into_iter()
            .enumerate()
    {
        let revision = 4 + offset as u64;
        let mut other = grouped_snapshot(standard_agents());
        other.revision = revision;
        state.set_snapshot(Box::new(other));
        announce(&mut state, source, Some(token));
        state.set_pane_surface({
            let mut surface = surface();
            surface.projection_revision = revision;
            surface
        });
        state.compose(106, 30).expect("other view frame");
        assert_eq!(state.hits.agent_group_headings.len(), headings);
        assert_eq!(
            visible_agents(&state).len(),
            7,
            "no fold carries across views"
        );
    }
}

#[test]
fn projection_before_the_snapshot_of_a_new_boot_applies_and_stale_projection_does_not() {
    let (mut state, _) = grouped_state(standard_agents(), Some(TOKEN));
    let beta = state.hits.agent_group_headings[1].0;
    click(&mut state, beta);
    state.compose(106, 30).expect("beta folded");
    assert!(!state.agent_group_folds.is_empty());
    let revision = state.snapshot.as_deref().expect("snapshot").revision;

    // The server sends boot-2's projection first; boot-1 keeps its own grouping.
    state.set_test_endpoint_agent_view_projection(
        &ClientEndpointId::Local,
        "boot-2",
        revision,
        Some(view(SOURCE, Some(TOKEN))),
    );
    let frame = state.compose(106, 30).expect("boot-1 frame");
    assert_eq!(heading_rows(&state, &frame).len(), 3);

    let mut next_boot = grouped_snapshot(standard_agents());
    next_boot.boot_id = "boot-2".into();
    state.set_snapshot(Box::new(next_boot));
    let mut surface = surface();
    surface.boot_id = "boot-2".into();
    state.set_pane_surface(surface);
    let frame = state.compose(106, 30).expect("new boot frame");
    assert_eq!(
        heading_rows(&state, &frame),
        vec!["▾ alpha 3 !1", "▾ beta 2", "▾ Other 2"],
        "projection received before the snapshot groups it, with folds reset for the new boot"
    );
    assert!(state.agent_group_folds.is_empty());

    // A late projection from boot-1 must not group or fold anything of boot-2.
    state.set_test_endpoint_agent_view_projection(
        &ClientEndpointId::Local,
        "boot-1",
        revision,
        Some(view(SOURCE, Some("stale_token"))),
    );
    let frame = state.compose(106, 30).expect("stale projection frame");
    assert_eq!(heading_rows(&state, &frame).len(), 3);
    assert!(state.agent_group_folds.is_empty());
}

#[test]
fn boot_change_drops_fold_state_and_the_old_boots_grouping() {
    let (mut state, _) = grouped_state(standard_agents(), Some(TOKEN));
    let alpha = state.hits.agent_group_headings[0].0;
    click(&mut state, alpha);
    state.compose(106, 30).expect("alpha folded");
    assert_eq!(state.agent_group_folds.len(), 1);

    let mut rebooted = grouped_snapshot(standard_agents());
    rebooted.boot_id = "boot-2".into();
    state.set_snapshot(Box::new(rebooted));
    assert!(state.agent_group_folds.is_empty(), "folds are per boot");
    let mut surface = surface();
    surface.boot_id = "boot-2".into();
    state.set_pane_surface(surface);
    let frame = state.compose(106, 30).expect("flat after reboot");
    assert!(heading_rows(&state, &frame).is_empty());
    assert_eq!(visible_agents(&state).len(), 7);
}

#[test]
fn mixed_versions_show_a_flat_list_and_send_no_restore() {
    // Fork client + upstream server: no agent-view projection support at all, or a projection
    // whose view has no group_by. Both render the flat list and no heading is a click target.
    let agents = standard_agents();
    for supported in [false, true] {
        let mut state = ClientShellState::new(shell_config());
        state.set_snapshot(Box::new(grouped_snapshot(agents.clone())));
        if supported {
            announce(&mut state, SOURCE, None);
        } else {
            state.set_endpoint_agent_view_projection_supported(&ClientEndpointId::Local, false);
        }
        state.set_pane_surface(surface());
        let frame = state.compose(106, 30).expect("flat frame");
        assert!(heading_rows(&state, &frame).is_empty());
        assert!(state.hits.agent_group_headings.is_empty());
        assert_eq!(visible_agents(&state).len(), 7);
    }

    // Restore is never an endpoint request; the shell only queues an API-socket action, which
    // the runtime sends only when the server advertises `agent_view_restore`.
    let mut config = Config::default();
    config.ui.agent_panel_sort = crate::config::AgentPanelSortConfig::Priority;
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&config));
    state.set_snapshot(Box::new(snapshot()));
    state.set_pane_surface(surface());
    state.compose(106, 30).expect("priority frame");
    let toggle = state.hits.agent_sort_toggle;
    let input = click(&mut state, toggle);
    assert!(
        input
            .actions
            .iter()
            .all(|action| !matches!(action, ClientShellAction::Endpoint { .. })),
        "restore must never be an endpoint request"
    );
}

// Manual scaling profile. `just bench-render-scale` runs it through its `render_scale_profile`
// filter. It builds in-memory snapshots only: no PTYs, servers, or sockets.
const BENCH_COLS: u16 = 120;
const BENCH_ROWS: u16 = 40;
const BENCH_SAMPLES: usize = 40;
const BENCH_WARMUP: usize = 5;
const BENCH_CARDINALITIES: [usize; 3] = [1, 15, 50];
const BENCH_PROJECTS: usize = 5;

#[derive(Clone, Copy, Debug)]
enum BenchMode {
    Flat,
    Grouped,
    Folded,
}

impl BenchMode {
    fn name(self) -> &'static str {
        match self {
            BenchMode::Flat => "flat",
            BenchMode::Grouped => "grouped",
            BenchMode::Folded => "folded",
        }
    }
}

/// Number of group headings a mode must draw for `agents` agents.
fn expected_headings(mode: BenchMode, agents: usize) -> usize {
    match mode {
        BenchMode::Flat => 0,
        BenchMode::Grouped | BenchMode::Folded => agents.min(BENCH_PROJECTS),
    }
}

fn bench_state(agents: usize, mode: BenchMode) -> ClientShellState {
    let members = (0..agents)
        .map(|index| {
            agent(
                &format!("pane_b{index}"),
                // Agents are generated project by project so server order keeps groups contiguous.
                Some(&format!("project-{}", index * BENCH_PROJECTS / agents)),
                match index % 3 {
                    0 => AgentStatus::Blocked,
                    1 => AgentStatus::Working,
                    _ => AgentStatus::Idle,
                },
            )
        })
        .collect::<Vec<_>>();
    let mut state = ClientShellState::new(shell_config());
    state.set_agent_view_group_by_supported(true);
    let mut projected = grouped_snapshot(members);
    projected.boot_id = "bench-boot".into();
    state.set_snapshot(Box::new(projected));
    announce(
        &mut state,
        SOURCE,
        match mode {
            BenchMode::Flat => None,
            _ => Some(TOKEN),
        },
    );
    let mut surface = surface();
    surface.boot_id = "bench-boot".into();
    state.set_pane_surface(surface);
    state.compose(BENCH_COLS, BENCH_ROWS).expect("first frame");
    if matches!(mode, BenchMode::Folded) {
        // Fold every group, not only the headings that fit the viewport of the first frame.
        let keys = {
            let snapshot = state.snapshot.as_deref().expect("snapshot");
            let ordered = super::agent_sidebar::ordered_agent_pane_ids(
                snapshot,
                state.config.agent_panel_sort,
            );
            super::agent_sidebar::agent_groups(snapshot, &ordered, TOKEN)
                .into_iter()
                .map(|group| group.key)
                .collect::<Vec<_>>()
        };
        for key in keys {
            state.toggle_agent_group(&key);
        }
    }
    state
}

fn percentile_us(sorted: &[Duration], percentile: usize) -> u128 {
    sorted[(sorted.len() * percentile / 100).min(sorted.len() - 1)].as_micros()
}

#[test]
#[ignore = "manual grouped agent sidebar render scaling profile"]
fn grouped_agent_render_scale_profile() {
    println!("grouped agent sidebar compose, {BENCH_COLS}x{BENCH_ROWS}, {BENCH_SAMPLES} samples");
    println!("  agents  mode     headings  median_us  p95_us  max_us");
    for agents in BENCH_CARDINALITIES {
        for mode in [BenchMode::Flat, BenchMode::Grouped, BenchMode::Folded] {
            let mut state = bench_state(agents, mode);
            for _ in 0..BENCH_WARMUP {
                black_box(state.compose(BENCH_COLS, BENCH_ROWS));
            }
            let mut samples = Vec::with_capacity(BENCH_SAMPLES);
            for _ in 0..BENCH_SAMPLES {
                let started = Instant::now();
                black_box(
                    state
                        .compose(BENCH_COLS, BENCH_ROWS)
                        .expect("benchmark frame"),
                );
                samples.push(started.elapsed());
            }
            // Correctness is asserted outside the timed region.
            let snapshot = state.snapshot.as_deref().expect("snapshot");
            let ordered = super::agent_sidebar::ordered_agent_pane_ids(
                snapshot,
                state.config.agent_panel_sort,
            );
            let groups = super::agent_sidebar::agent_groups(snapshot, &ordered, TOKEN).len();
            let headings = state.hits.agent_group_headings.len();
            match mode {
                BenchMode::Flat => assert_eq!(headings, 0, "{agents} {mode:?}"),
                // A tall list scrolls, so only the headings in view are hit targets.
                BenchMode::Grouped => {
                    assert!((1..=expected_headings(mode, agents)).contains(&headings));
                    assert!(!state.hits.agents.is_empty());
                }
                BenchMode::Folded => {
                    assert_eq!(
                        headings,
                        expected_headings(mode, agents),
                        "{agents} {mode:?}"
                    );
                    assert!(state.hits.agents.is_empty(), "folded groups hide agents");
                }
            }
            if !matches!(mode, BenchMode::Flat) {
                assert_eq!(groups, expected_headings(mode, agents), "{agents} {mode:?}");
            }
            samples.sort();
            println!(
                "  {agents:>6}  {:<7}  {headings:>8}  {:>9}  {:>6}  {:>6}",
                mode.name(),
                percentile_us(&samples, 50),
                percentile_us(&samples, 95),
                samples[samples.len() - 1].as_micros(),
            );
        }
    }
}
