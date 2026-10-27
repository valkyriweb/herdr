use crate::api::schema::{ExternalAgentParams, Method, Request};
use std::io::{self, Read};

pub(super) fn run(args: &[String]) -> io::Result<i32> {
    let params = match args {
        [action] if action == "list" => ExternalAgentParams::List,
        [action, target] if action == "status" => ExternalAgentParams::Status {
            target: target.clone(),
        },
        [action, target] if action == "read" => ExternalAgentParams::Read {
            target: target.clone(),
            after_position: None,
        },
        [action, target, cursor] if action == "read" => ExternalAgentParams::Read {
            target: target.clone(),
            after_position: Some(
                cursor
                    .parse()
                    .map_err(|_| io::Error::other("invalid timeline cursor"))?,
            ),
        },
        [action, target, key] if action == "prompt" => {
            let mut text = String::new();
            io::stdin().take(120_001).read_to_string(&mut text)?;
            ExternalAgentParams::Prompt {
                target: target.clone(),
                text,
                client_request_id: key.clone(),
            }
        }
        _ => {
            eprintln!("usage: herdr agent external list | status <id> | read <id> [cursor] | prompt <id> <clientRequestId> (text on stdin)");
            return Ok(2);
        }
    };
    params.validate().map_err(io::Error::other)?;
    super::print_response(&super::send_request(&Request {
        id: "cli:agent:external".into(),
        method: Method::ExternalAgent(params),
    })?)
}
