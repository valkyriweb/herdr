use serde::{Deserialize, Serialize};

/// Runtime-backed agents have their own IDs and do not require terminal placement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum ExternalAgentParams {
    List,
    Start {
        title: String,
        #[serde(default, skip_serializing_if = "Option::is_none", rename = "projectId")]
        project_id: Option<String>,
        #[serde(default)]
        scratch: bool,
        #[serde(rename = "modelSelection")]
        model_selection: ExternalAgentModel,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        text: Option<String>,
    },
    Stop {
        target: String,
        #[serde(rename = "clientRequestId")]
        client_request_id: String,
    },
    Resume {
        target: String,
        #[serde(rename = "clientRequestId")]
        client_request_id: String,
    },
    Read {
        target: String,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            rename = "afterPosition"
        )]
        after_position: Option<u64>,
    },
    Status {
        target: String,
    },
    Prompt {
        target: String,
        text: String,
        #[serde(rename = "clientRequestId")]
        client_request_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExternalAgentModel {
    #[serde(rename = "instanceId")]
    pub instance_id: String,
    pub model: String,
}

impl ExternalAgentParams {
    pub(crate) fn validate(&self) -> Result<(), &'static str> {
        let target = match self {
            Self::List => return Ok(()),
            Self::Start {
                title,
                project_id,
                scratch,
                model_selection,
                text,
            } => {
                if title.trim().is_empty() || title.len() > 256 {
                    return Err("title must contain 1–256 bytes");
                }
                if *scratch == project_id.is_some()
                    || project_id.as_ref().is_some_and(|id| id.trim().is_empty())
                {
                    return Err("choose exactly one projectId or scratch");
                }
                if model_selection.instance_id.trim().is_empty()
                    || model_selection.model.trim().is_empty()
                {
                    return Err("provider instance and model are required");
                }
                if let Some(text) = text {
                    validate_prompt(text)?;
                }
                return Ok(());
            }
            Self::Stop {
                target,
                client_request_id,
            }
            | Self::Resume {
                target,
                client_request_id,
            } => {
                validate_request_id(client_request_id)?;
                target
            }
            Self::Read { target, .. } | Self::Status { target } => target,
            Self::Prompt {
                target,
                text,
                client_request_id,
            } => {
                validate_prompt(text)?;
                validate_request_id(client_request_id)?;
                target
            }
        };
        if target
            .strip_prefix("t3:")
            .is_none_or(|id| id.trim().is_empty())
        {
            return Err("expected t3:<thread> agent ID");
        }
        Ok(())
    }
}

fn validate_prompt(text: &str) -> Result<(), &'static str> {
    if text.trim().is_empty() || text.len() > 120_000 {
        return Err("prompt text must contain 1–120000 bytes");
    }
    Ok(())
}

fn validate_request_id(id: &str) -> Result<(), &'static str> {
    if id.trim().is_empty() || id.len() > 256 {
        return Err("a stable clientRequestId of at most 256 bytes is required");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lifecycle_requires_a_destination_and_stable_mutation_keys() {
        for action in ["stop", "resume"] {
            let mut input = serde_json::json!({"action":action,"target":"t3:opaque:thread","clientRequestId":"stable"});
            assert!(serde_json::from_value::<ExternalAgentParams>(input.clone())
                .unwrap()
                .validate()
                .is_ok());
            input["clientRequestId"] = serde_json::json!("");
            assert!(serde_json::from_value::<ExternalAgentParams>(input)
                .unwrap()
                .validate()
                .is_err());
        }
        for (project, scratch, valid) in [
            (None, false, false),
            (None, true, true),
            (Some("p1"), false, true),
            (Some("p1"), true, false),
            (Some(" "), false, false),
        ] {
            let params = ExternalAgentParams::Start {
                title: "worker".into(),
                project_id: project.map(str::to_owned),
                scratch,
                model_selection: ExternalAgentModel {
                    instance_id: "codex".into(),
                    model: "fixture".into(),
                },
                text: None,
            };
            assert_eq!(params.validate().is_ok(), valid);
        }
    }

    #[test]
    fn prompts_require_a_retry_key_and_keep_opaque_thread_ids() {
        let input = serde_json::json!({"action":"prompt", "target":"t3:thread:provider:abc", "text":"hello", "clientRequestId":"stable"});
        let params: ExternalAgentParams = serde_json::from_value(input.clone()).unwrap();
        assert_eq!(params.validate(), Ok(()));
        assert_eq!(serde_json::to_value(params).unwrap(), input);
        assert!(serde_json::from_value::<ExternalAgentParams>(
            serde_json::json!({"action":"prompt", "target":"t3:a", "text":"hi"})
        )
        .is_err());
        assert!(serde_json::from_value::<ExternalAgentParams>(
            serde_json::json!({"action":"focus", "target":"t3:a"})
        )
        .is_err());
    }
}
