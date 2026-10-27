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

pub(super) fn handles(args: &[String]) -> bool {
    args.first().is_some_and(|action| action != "external")
        && (args.get(1).is_some_and(|target| target.starts_with("t3:"))
            || (args.first().is_some_and(|action| action == "start")
                && args.windows(2).any(|pair| pair == ["--kind", "t3"])))
}

pub(super) fn run_native(args: &[String]) -> io::Result<i32> {
    let params = native_params(args).map_err(io::Error::other)?;
    params.validate().map_err(io::Error::other)?;
    super::print_response(&super::send_request(&Request {
        id: "cli:agent:t3".into(),
        method: Method::ExternalAgent(params),
    })?)
}

fn native_params(args: &[String]) -> Result<ExternalAgentParams, String> {
    use crate::api::schema::ExternalAgentModel;

    let action = args.first().ok_or("missing action")?.as_str();
    let target = args
        .get(1)
        .ok_or("missing T3 agent target or title")?
        .clone();
    if action == "start" {
        let mut project_id = None;
        let mut scratch = false;
        let mut instance_id = None;
        let mut model = None;
        let mut text = None;
        let mut options = args[2..].iter();
        while let Some(option) = options.next() {
            if option == "--scratch" {
                scratch = true;
                continue;
            }
            let value = options
                .next()
                .ok_or_else(|| format!("missing value for {option}"))?;
            match option.as_str() {
                "--kind" if value == "t3" => {},
                "--project" => project_id = Some(value.clone()),
                "--provider" => instance_id = Some(value.clone()),
                "--model" => model = Some(value.clone()),
                "--text" => text = Some(value.clone()),
                _ => return Err(format!("T3 start does not support {option}; use --project or --scratch, --provider and --model")),
            }
        }
        return Ok(ExternalAgentParams::Start {
            title: target,
            project_id,
            scratch,
            model_selection: ExternalAgentModel {
                instance_id: instance_id.ok_or("T3 start requires --provider INSTANCE")?,
                model: model.ok_or("T3 start requires --model MODEL")?,
            },
            text,
        });
    }
    match action {
        "get" | "status" if args.len() == 2 => Ok(ExternalAgentParams::Status { target }),
        "read" => {
            let mut after_position = None;
            let mut options = args[2..].iter();
            while let Some(option) = options.next() {
                let value = options.next().ok_or_else(|| format!("missing value for {option}"))?;
                match (option.as_str(), value.as_str()) {
                    ("--after-position", value) => after_position = Some(value.parse().map_err(|_| "invalid timeline cursor")?),
                    ("--source", "recent") | ("--format", "text") => {},
                    _ => return Err(format!("T3 read does not support {option} {value}; use --after-position CURSOR to continue the timeline")),
                }
            }
            Ok(ExternalAgentParams::Read { target, after_position })
        }
        "prompt" => {
            let text = args.get(2).ok_or("missing prompt text")?.clone();
            Ok(ExternalAgentParams::Prompt {
                target,
                text,
                client_request_id: request_key(&args[3..])?,
            })
        }
        "stop" => Ok(ExternalAgentParams::Stop { target, client_request_id: request_key(&args[2..])? }),
        "resume" => Ok(ExternalAgentParams::Resume { target, client_request_id: request_key(&args[2..])? }),
        _ => Err(format!("T3 does not support agent {action} with these options; use get, status, read, prompt, stop or resume")),
    }
}

fn request_key(options: &[String]) -> Result<String, String> {
    match options {
        [flag, key] if flag == "--client-request-id" => Ok(key.clone()),
        _ => Err("T3 mutations require --client-request-id ID; use status to observe completion (terminal wait options are unsupported)".into()),
    }
}
