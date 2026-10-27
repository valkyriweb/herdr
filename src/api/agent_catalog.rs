use super::schema::{
    AgentBackendDiagnostic, AgentCatalogEntry, AgentInfo, AgentPlacement, AgentStatus,
    ExternalAgentParams, ResponseResult,
};

pub(super) fn collect(terminals: Vec<AgentInfo>) -> ResponseResult {
    let external = match super::external_agents::configured_executable() {
        Ok(None) => None,
        Ok(Some(_)) => Some(super::external_agents::execute(&ExternalAgentParams::List)),
        Err(error) => Some(Err(error)),
    };
    combine(terminals, external)
}

fn combine(
    terminals: Vec<AgentInfo>,
    external: Option<Result<serde_json::Value, String>>,
) -> ResponseResult {
    let mut agents = terminals
        .into_iter()
        .map(terminal_entry)
        .collect::<Vec<_>>();
    let mut diagnostics = Vec::new();
    if let Some(external) = external {
        match external.and_then(t3_entries) {
            Ok(entries) => agents.extend(entries),
            Err(message) => diagnostics.push(AgentBackendDiagnostic {
                backend: "t3".into(),
                message,
            }),
        }
    }
    ResponseResult::AgentCatalog {
        agents,
        diagnostics,
    }
}

fn terminal_entry(agent: AgentInfo) -> AgentCatalogEntry {
    AgentCatalogEntry {
        id: agent.terminal_id.clone(),
        backend: "terminal".into(),
        name: agent
            .name
            .or(agent.title)
            .unwrap_or_else(|| agent.terminal_id.clone()),
        status: agent.agent_status,
        capabilities: ["read", "prompt", "status", "send_keys", "focus"]
            .map(str::to_owned)
            .to_vec(),
        placement: Some(AgentPlacement {
            terminal_id: agent.terminal_id,
            workspace_id: agent.workspace_id,
            tab_id: agent.tab_id,
            pane_id: agent.pane_id,
        }),
        provider: agent.agent,
        model: None,
        cwd: agent.cwd,
    }
}

fn t3_entries(value: serde_json::Value) -> Result<Vec<AgentCatalogEntry>, String> {
    let entries = value["agents"]
        .as_array()
        .ok_or("T3 adapter returned an invalid agent catalog")?;
    entries
        .iter()
        .map(|entry| {
            let id = entry["id"]
                .as_str()
                .filter(|id| {
                    id.strip_prefix("t3:")
                        .is_some_and(|thread| !thread.is_empty())
                })
                .ok_or("T3 adapter returned an invalid agent ID")?;
            let status = match entry["status"].as_str() {
                _ if entry["pendingRequestCount"].as_u64().unwrap_or(0) > 0 => AgentStatus::Blocked,
                Some("waiting") => AgentStatus::Blocked,
                Some("queued" | "preparing" | "starting" | "running") => AgentStatus::Working,
                Some("completed") => AgentStatus::Done,
                Some("idle" | "stopped" | "interrupted") => AgentStatus::Idle,
                _ => AgentStatus::Unknown,
            };
            Ok(AgentCatalogEntry {
                id: id.into(),
                backend: "t3".into(),
                name: entry["title"].as_str().unwrap_or(id).into(),
                status,
                capabilities: entry["capabilities"]
                    .as_array()
                    .ok_or("T3 adapter omitted agent capabilities")?
                    .iter()
                    .map(|value| {
                        value
                            .as_str()
                            .map(str::to_owned)
                            .ok_or("invalid T3 capability".into())
                    })
                    .collect::<Result<_, String>>()?,
                placement: None,
                provider: entry["providerInstanceId"].as_str().map(str::to_owned),
                model: entry["model"].as_str().map(str::to_owned),
                cwd: entry["worktreePath"].as_str().map(str::to_owned),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_keeps_terminal_agents_when_t3_is_unavailable() {
        let terminal: AgentInfo = serde_json::from_value(serde_json::json!({
            "terminal_id":"term-1","name":"builder","agent_status":"working",
            "workspace_id":"w1","tab_id":"t1","pane_id":"p1","focused":false,"revision":1
        }))
        .unwrap();
        let ResponseResult::AgentCatalog {
            agents,
            diagnostics,
        } = combine(vec![terminal], Some(Err("T3 unavailable".into())))
        else {
            panic!("expected catalog");
        };
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].id, "term-1");
        assert_eq!(agents[0].placement.as_ref().unwrap().pane_id, "p1");
        assert_eq!(diagnostics[0].message, "T3 unavailable");
    }

    #[test]
    fn catalog_maps_t3_status_without_inventing_terminal_placement() {
        let ResponseResult::AgentCatalog {
            agents,
            diagnostics,
        } = combine(
            vec![],
            Some(Ok(serde_json::json!({"agents":[{
                "id":"t3:thread:1","title":"worker","status":"running",
                "pendingRequestCount":1,"capabilities":["read","prompt","status","stop","resume"],
                "providerInstanceId":"codex","model":"test-model"
            }]}))),
        )
        else {
            panic!("expected catalog");
        };
        assert!(diagnostics.is_empty());
        assert_eq!(agents[0].status, AgentStatus::Blocked);
        assert_eq!(agents[0].placement, None);
        assert_eq!(agents[0].id, "t3:thread:1");
    }
}
