use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::schema::ExternalAgentParams;

const MAX_OUTPUT: u64 = 8 * 1024 * 1024;

pub(super) fn execute(params: &ExternalAgentParams) -> Result<serde_json::Value, String> {
    params.validate().map_err(str::to_owned)?;
    let executable = std::env::var_os("HERDR_T3_ADAPTER").ok_or_else(|| {
        "external agents are disabled; set HERDR_T3_ADAPTER to an approved adapter executable"
            .to_owned()
    })?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| "could not start external adapter runtime".to_owned())?;
    runtime.block_on(invoke(Path::new(&executable), params))
}

async fn invoke(
    executable: &Path,
    params: &ExternalAgentParams,
) -> Result<serde_json::Value, String> {
    if !executable.is_absolute() {
        return Err("HERDR_T3_ADAPTER must be an absolute executable path".to_owned());
    }
    let input =
        serde_json::to_vec(params).map_err(|_| "could not encode adapter request".to_owned())?;
    let mut child = tokio::process::Command::new(executable)
        .args(["t3", "request"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| "could not start configured T3 adapter".to_owned())?;
    let operation = async {
        let mut stdin = child.stdin.take().ok_or("adapter stdin unavailable")?;
        let stdout = child.stdout.take().ok_or("adapter stdout unavailable")?;
        let write = async {
            stdin.write_all(&input).await.map_err(|_| {
                "adapter input failed; check the configured executable and approved credential"
            })?;
            stdin
                .shutdown()
                .await
                .map_err(|_| "adapter stdin close failed; check the configured executable and approved credential")?;
            drop(stdin);
            Ok::<(), &str>(())
        };
        let read = async {
            let mut output = Vec::new();
            stdout
                .take(MAX_OUTPUT + 1)
                .read_to_end(&mut output)
                .await
                .map_err(|_| "adapter output failed")?;
            if output.len() as u64 > MAX_OUTPUT {
                return Err("adapter response exceeds 8 MiB");
            }
            Ok(output)
        };
        let (_, output) = tokio::try_join!(write, read)?;
        let status = child.wait().await.map_err(|_| "adapter wait failed")?;
        if !status.success() {
            return Err("T3 adapter failed; check its configuration and approved credential");
        }
        let value: serde_json::Value =
            serde_json::from_slice(&output).map_err(|_| "adapter returned invalid JSON")?;
        if !value.is_object() {
            return Err("adapter response must be an object");
        }
        Ok(value)
    };
    match tokio::time::timeout(Duration::from_secs(120), operation).await {
        Ok(result) => result.map_err(str::to_owned),
        Err(_) => {
            let _ = child.kill().await;
            Err("T3 adapter timed out; retry prompts only with the same clientRequestId".to_owned())
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn adapter_receives_json_without_a_pane_or_shell_interpolation() {
        let directory =
            std::env::temp_dir().join(format!("herdr-t3-fixture-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let executable = directory.join("fixture");
        std::fs::write(
            &executable,
            "#!/bin/sh\n[ \"$1\" = t3 ] && [ \"$2\" = request ] || exit 2\ncat\n",
        )
        .unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let params = ExternalAgentParams::Prompt {
            target: "t3:thread:fixture".into(),
            text: "literal $(echo dangerous)".into(),
            client_request_id: "stable-key".into(),
        };
        let result = runtime.block_on(invoke(&executable, &params)).unwrap();
        assert_eq!(result, serde_json::to_value(&params).unwrap());
        std::fs::remove_dir_all(directory).unwrap();
    }
}
