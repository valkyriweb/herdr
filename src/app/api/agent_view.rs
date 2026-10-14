use crate::api::schema::{AgentViewClearParams, AgentViewSetParams, ResponseResult};
use crate::app::App;

use super::responses::{encode_error, encode_success};

impl App {
    pub(super) fn handle_agent_view_set(
        &mut self,
        id: String,
        mut params: AgentViewSetParams,
    ) -> String {
        if let Err(message) = crate::app::agent_view::validate_agent_view(&mut params) {
            return encode_error(id, "invalid_agent_view", message);
        }
        if let Some(plugin_id) = params.source.strip_prefix("plugin:") {
            let Some(plugin_id) = super::plugins::normalize_plugin_id(plugin_id) else {
                return encode_error(
                    id,
                    "invalid_agent_view",
                    "plugin-owned agent view source has an invalid plugin id",
                );
            };
            let Some(plugin) = self.state.installed_plugins.get(&plugin_id) else {
                return encode_error(id, "plugin_not_found", "plugin not found");
            };
            if !plugin.enabled {
                return encode_error(id, "plugin_disabled", "plugin is disabled");
            }
        }
        if params.source.starts_with("plugin:") && params.filter.is_none() {
            self.state.last_plugin_agent_view = Some(params.clone());
        }
        let source = params.source.clone();
        let label = params.label.clone();
        self.replace_agent_view_override(Some(params));
        encode_success(
            id,
            ResponseResult::AgentView {
                active: true,
                source: Some(source),
                label,
            },
        )
    }

    pub(super) fn handle_agent_view_restore(&mut self, id: String) -> String {
        if let Some(view) = self.state.last_plugin_agent_view.clone() {
            let plugin_enabled = view
                .source
                .strip_prefix("plugin:")
                .and_then(super::plugins::normalize_plugin_id)
                .and_then(|plugin_id| self.state.installed_plugins.get(&plugin_id))
                .is_some_and(|plugin| plugin.enabled);
            if view.filter.is_none() && plugin_enabled {
                let source = view.source.clone();
                let label = view.label.clone();
                self.replace_agent_view_override(Some(view));
                return encode_success(
                    id,
                    ResponseResult::AgentView {
                        active: true,
                        source: Some(source),
                        label,
                    },
                );
            }
            self.state.last_plugin_agent_view = None;
        }
        let active = self.state.agent_view_override.as_ref();
        encode_success(
            id,
            ResponseResult::AgentView {
                active: active.is_some(),
                source: active.map(|view| view.source.clone()),
                label: active.and_then(|view| view.label.clone()),
            },
        )
    }

    pub(super) fn handle_agent_view_clear(
        &mut self,
        id: String,
        params: AgentViewClearParams,
    ) -> String {
        let source = match params.source {
            Some(source) => match crate::app::agent_view::validate_agent_view_source(&source) {
                Ok(source) => Some(source),
                Err(message) => return encode_error(id, "invalid_agent_view", message),
            },
            None => None,
        };
        if source.as_deref().is_none_or(|source| {
            self.state
                .agent_view_override
                .as_ref()
                .is_some_and(|active| active.source == source)
        }) {
            self.replace_agent_view_override(None);
        }
        let active = self.state.agent_view_override.as_ref();
        encode_success(
            id,
            ResponseResult::AgentView {
                active: active.is_some(),
                source: active.map(|view| view.source.clone()),
                label: active.and_then(|view| view.label.clone()),
            },
        )
    }

    pub(crate) fn clear_agent_view_for_source(&mut self, source: &str) -> bool {
        let active_cleared = self
            .state
            .agent_view_override
            .as_ref()
            .is_some_and(|active| active.source == source);
        if active_cleared {
            self.replace_agent_view_override(None);
        }
        let remembered_cleared = self
            .state
            .last_plugin_agent_view
            .as_ref()
            .is_some_and(|view| view.source == source);
        if remembered_cleared {
            self.state.last_plugin_agent_view = None;
        }
        active_cleared || remembered_cleared
    }

    fn replace_agent_view_override(&mut self, view: Option<AgentViewSetParams>) {
        self.state.agent_view_override = view;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::schema::{
        AgentViewBuiltinField, AgentViewField, AgentViewFilter, AgentViewValue,
    };

    fn test_app() -> App {
        let (_api_tx, api_rx) = tokio::sync::mpsc::unbounded_channel();
        App::new(
            &crate::config::Config::default(),
            crate::app::AppPolicy::TEST,
            None,
            api_rx,
            crate::api::EventHub::default(),
        )
    }

    fn working_view(source: &str) -> AgentViewSetParams {
        AgentViewSetParams {
            source: source.to_string(),
            label: Some("working".to_string()),
            filter: Some(AgentViewFilter::Eq {
                field: AgentViewField::Builtin(AgentViewBuiltinField::Status),
                value: AgentViewValue::String("working".to_string()),
            }),
            sort: Vec::new(),
        }
    }

    fn install_test_plugin(app: &mut App, enabled: bool) {
        let plugin_id = "example.worktree-bootstrap".to_string();
        app.state.installed_plugins.insert(
            plugin_id.clone(),
            crate::api::schema::InstalledPluginInfo {
                plugin_id,
                name: "Example".to_string(),
                version: "1.0.0".to_string(),
                min_herdr_version: String::new(),
                description: None,
                manifest_path: String::new(),
                plugin_root: String::new(),
                enabled,
                platforms: None,
                build: Vec::new(),
                startup: Vec::new(),
                actions: Vec::new(),
                events: Vec::new(),
                panes: Vec::new(),
                link_handlers: Vec::new(),
                source: Default::default(),
                warnings: Vec::new(),
            },
        );
    }

    fn projects_view(filter: Option<AgentViewFilter>) -> AgentViewSetParams {
        AgentViewSetParams {
            source: "plugin:example.worktree-bootstrap".to_string(),
            label: Some("projects".to_string()),
            filter,
            sort: Vec::new(),
        }
    }

    #[test]
    fn set_and_source_guarded_clear_replace_transient_view() {
        let mut app = test_app();

        let set = app.handle_agent_view_set("set".to_string(), working_view("example.views"));
        let set: crate::api::schema::SuccessResponse = serde_json::from_str(&set).unwrap();
        assert_eq!(
            set.result,
            ResponseResult::AgentView {
                active: true,
                source: Some("example.views".to_string()),
                label: Some("working".to_string()),
            }
        );
        assert_eq!(
            app.state
                .agent_view_override
                .as_ref()
                .map(|view| view.source.as_str()),
            Some("example.views")
        );

        app.handle_agent_view_clear(
            "wrong-source".to_string(),
            AgentViewClearParams {
                source: Some("other.views".to_string()),
            },
        );
        assert!(app.state.agent_view_override.is_some());

        app.handle_agent_view_clear(
            "right-source".to_string(),
            AgentViewClearParams {
                source: Some("example.views".to_string()),
            },
        );
        assert!(app.state.agent_view_override.is_none());
    }

    #[test]
    fn unfiltered_plugin_view_survives_clear_and_can_be_restored() {
        let mut app = test_app();
        install_test_plugin(&mut app, true);
        let expected = projects_view(None);

        app.handle_agent_view_set("set".to_string(), expected.clone());
        assert_eq!(app.state.last_plugin_agent_view, Some(expected.clone()));

        app.handle_agent_view_clear("clear".to_string(), AgentViewClearParams::default());
        assert!(app.state.agent_view_override.is_none());
        assert_eq!(app.state.last_plugin_agent_view, Some(expected.clone()));

        let response = app.handle_agent_view_restore("restore".to_string());
        let response: crate::api::schema::SuccessResponse =
            serde_json::from_str(&response).unwrap();
        assert_eq!(
            response.result,
            ResponseResult::AgentView {
                active: true,
                source: Some(expected.source.clone()),
                label: expected.label.clone(),
            }
        );
        assert_eq!(app.state.agent_view_override, Some(expected));
    }

    #[test]
    fn filtered_plugin_view_does_not_replace_remembered_unfiltered_view() {
        let mut app = test_app();
        install_test_plugin(&mut app, true);
        let expected = projects_view(None);
        app.handle_agent_view_set("set-unfiltered".to_string(), expected.clone());

        app.handle_agent_view_set(
            "set-filtered".to_string(),
            working_view("plugin:example.worktree-bootstrap"),
        );
        assert_eq!(app.state.last_plugin_agent_view, Some(expected.clone()));

        app.handle_agent_view_clear("clear".to_string(), AgentViewClearParams::default());
        app.handle_agent_view_restore("restore".to_string());
        assert_eq!(app.state.agent_view_override, Some(expected));
    }

    #[test]
    fn restore_discards_view_when_its_plugin_is_disabled() {
        let mut app = test_app();
        install_test_plugin(&mut app, true);
        let expected = projects_view(None);
        app.handle_agent_view_set("set".to_string(), expected);
        app.handle_agent_view_clear("clear".to_string(), AgentViewClearParams::default());
        app.state
            .installed_plugins
            .get_mut("example.worktree-bootstrap")
            .expect("test plugin")
            .enabled = false;

        let response = app.handle_agent_view_restore("restore".to_string());
        let response: crate::api::schema::SuccessResponse =
            serde_json::from_str(&response).unwrap();
        assert_eq!(
            response.result,
            ResponseResult::AgentView {
                active: false,
                source: None,
                label: None,
            }
        );
        assert!(app.state.last_plugin_agent_view.is_none());
        assert!(app.state.agent_view_override.is_none());
    }

    #[test]
    fn invalid_view_does_not_replace_active_view() {
        let mut app = test_app();
        app.handle_agent_view_set("set".to_string(), working_view("example.views"));

        let mut invalid = working_view("example.other");
        invalid.filter = Some(AgentViewFilter::Any {
            filters: Vec::new(),
        });
        let response = app.handle_agent_view_set("invalid".to_string(), invalid);
        let response: crate::api::schema::ErrorResponse = serde_json::from_str(&response).unwrap();

        assert_eq!(response.error.code, "invalid_agent_view");
        assert_eq!(
            app.state
                .agent_view_override
                .as_ref()
                .map(|view| view.source.as_str()),
            Some("example.views")
        );
    }
}
