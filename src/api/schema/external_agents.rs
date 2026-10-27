use serde::{Deserialize, Serialize};

/// Runtime-backed agents have their own IDs and do not require terminal placement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum ExternalAgentParams {
    List,
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

impl ExternalAgentParams {
    pub(crate) fn validate(&self) -> Result<(), &'static str> {
        let target = match self {
            Self::List => return Ok(()),
            Self::Read { target, .. } | Self::Status { target } => target,
            Self::Prompt {
                target,
                text,
                client_request_id,
            } => {
                if text.trim().is_empty() || text.len() > 120_000 {
                    return Err("prompt text must contain 1–120000 bytes");
                }
                if client_request_id.trim().is_empty() || client_request_id.len() > 256 {
                    return Err("a stable clientRequestId of at most 256 bytes is required");
                }
                target
            }
        };
        if !target
            .strip_prefix("t3:")
            .is_some_and(|id| !id.trim().is_empty())
        {
            return Err("expected t3:<thread> agent ID");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
