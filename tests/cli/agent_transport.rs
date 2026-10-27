use super::harness::*;

#[test]
fn agent_start_waits_through_unknown_then_rejects_blocked() {
    let base = unique_test_dir();
    fs::create_dir_all(&base).unwrap();
    let socket_path = base.join("herdr.sock");
    let listener = UnixListener::bind(&socket_path).unwrap();

    let server = thread::spawn(move || {
        let (mut pane_stream, pane_line) = accept_fake_cli_operation(&listener);
        let pane: serde_json::Value = serde_json::from_str(&pane_line).unwrap();
        assert_eq!(pane["method"], "pane.get");
        assert_eq!(pane["params"]["pane_id"], "w1:p1");
        writeln!(
            pane_stream,
            "{}",
            serde_json::json!({
                "id": pane["id"],
                "result": {
                    "type": "pane_info",
                    "pane": { "terminal_id": "term_1" }
                }
            })
        )
        .unwrap();
        pane_stream.flush().unwrap();

        let (mut start_stream, start_line) = accept_fake_cli_operation(&listener);
        let start: serde_json::Value = serde_json::from_str(&start_line).unwrap();
        assert_eq!(start["method"], "agent.start");
        writeln!(
            start_stream,
            "{}",
            serde_json::json!({
                "id": start["id"],
                "result": {
                    "type": "agent_started",
                    "agent": {
                        "pane_id": "w1:p1",
                        "terminal_id": "term_1",
                        "name": "reviewer"
                    },
                    "argv": ["opencode"]
                }
            })
        )
        .unwrap();
        start_stream.flush().unwrap();

        let (mut get_stream, get_line) = accept_fake_cli_operation(&listener);
        let get: serde_json::Value = serde_json::from_str(&get_line).unwrap();
        assert_eq!(get["method"], "agent.get");
        assert_eq!(get["params"]["target"], "reviewer");
        writeln!(
            get_stream,
            "{}",
            serde_json::json!({
                "id": get["id"],
                "result": {
                    "type": "agent_info",
                    "agent": {
                        "agent": null,
                        "agent_status": "unknown",
                        "interactive_ready": true,
                        "launch_pending": false,
                        "name": "reviewer",
                        "pane_id": "w1:p1",
                        "terminal_id": "term_1"
                    }
                }
            })
        )
        .unwrap();
        get_stream.flush().unwrap();

        let (mut get_stream, get_line) = accept_fake_cli_operation(&listener);
        let get: serde_json::Value = serde_json::from_str(&get_line).unwrap();
        assert_eq!(get["method"], "agent.get");
        assert_eq!(get["params"]["target"], "reviewer");
        writeln!(
            get_stream,
            "{}",
            serde_json::json!({
                "id": get["id"],
                "result": {
                    "type": "agent_info",
                    "agent": {
                        "agent": "opencode",
                        "agent_status": "blocked",
                        "interactive_ready": true,
                        "launch_pending": false,
                        "name": "reviewer",
                        "pane_id": "w1:p1",
                        "terminal_id": "term_1"
                    }
                }
            })
        )
        .unwrap();
        get_stream.flush().unwrap();
    });

    let started = run_cli(
        &socket_path,
        &[
            "agent", "start", "reviewer", "--kind", "opencode", "--pane", "w1:p1",
        ],
    );
    assert_eq!(started.status.code(), Some(1));
    assert!(started.stdout.is_empty());
    let error: serde_json::Value = serde_json::from_slice(&started.stderr).unwrap();
    assert_eq!(error["error"]["code"], "agent_not_ready");

    server.join().unwrap();
    cleanup_test_base(&base);
}

#[test]
fn agent_start_does_not_retry_after_the_target_terminal_changes() {
    let base = unique_test_dir();
    fs::create_dir_all(&base).unwrap();
    let socket_path = base.join("herdr.sock");
    let listener = UnixListener::bind(&socket_path).unwrap();

    let server = thread::spawn(move || {
        for _ in 0..2 {
            let (mut stream, line) = accept_fake_cli_operation(&listener);
            let request: serde_json::Value = serde_json::from_str(&line).unwrap();
            assert_eq!(request["method"], "pane.get");
            writeln!(
                stream,
                "{}",
                serde_json::json!({
                    "id": request["id"],
                    "result": {
                        "type": "pane_info",
                        "pane": { "terminal_id": "term_1" }
                    }
                })
            )
            .unwrap();
            stream.flush().unwrap();

            let (mut stream, line) = accept_fake_cli_operation(&listener);
            let request: serde_json::Value = serde_json::from_str(&line).unwrap();
            if request["method"] == "agent.start" {
                writeln!(
                    stream,
                    "{}",
                    serde_json::json!({
                        "id": request["id"],
                        "error": {
                            "code": "agent_pane_busy",
                            "message": "agent target pane w1:p1 is not an available shell"
                        }
                    })
                )
                .unwrap();
                stream.flush().unwrap();
            } else {
                assert_eq!(request["method"], "pane.process_info");
                writeln!(
                    stream,
                    "{}",
                    serde_json::json!({
                        "id": request["id"],
                        "result": {
                            "type": "pane_process_info",
                            "process_info": {
                                "pane_id": "w1:p1",
                                "shell_pid": 10,
                                "foreground_process_group_id": 10,
                                "foreground_processes": [
                                    { "pid": 10, "name": "bash" },
                                    { "pid": 11, "name": "startup-helper" }
                                ]
                            }
                        }
                    })
                )
                .unwrap();
                stream.flush().unwrap();
            }
        }

        let (mut stream, line) = accept_fake_cli_operation(&listener);
        let request: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(request["method"], "pane.get");
        writeln!(
            stream,
            "{}",
            serde_json::json!({
                "id": request["id"],
                "result": {
                    "type": "pane_info",
                    "pane": { "terminal_id": "term_2" }
                }
            })
        )
        .unwrap();
        stream.flush().unwrap();
    });

    let started = run_cli(
        &socket_path,
        &[
            "agent", "start", "reviewer", "--kind", "pi", "--pane", "w1:p1",
        ],
    );
    assert_eq!(started.status.code(), Some(1));
    let error: serde_json::Value = serde_json::from_slice(&started.stderr).unwrap();
    assert_eq!(error["error"]["code"], "agent_pane_busy");

    server.join().unwrap();
    cleanup_test_base(&base);
}

#[test]
fn prompt_wait_is_sent_as_one_agent_request() {
    let base = unique_test_dir();
    fs::create_dir_all(&base).unwrap();
    let socket_path = base.join("herdr.sock");
    let listener = UnixListener::bind(&socket_path).unwrap();

    let server = thread::spawn(move || {
        let (mut prompt_stream, prompt_line) = accept_fake_cli_operation(&listener);
        let prompt: serde_json::Value = serde_json::from_str(&prompt_line).unwrap();
        assert_eq!(prompt["method"], "agent.prompt");
        assert_eq!(prompt["params"]["target"], "w1:p1");
        assert_eq!(
            prompt["params"]["wait"]["until"],
            serde_json::json!(["idle"])
        );
        assert!(prompt["params"]["wait"].get("timeout_ms").is_none());
        writeln!(
            prompt_stream,
            "{}",
            serde_json::json!({
                "id": prompt["id"],
                "result": {
                    "type": "agent_prompted",
                    "agent": {
                        "pane_id": "w1:p1",
                        "terminal_id": "term_1",
                        "name": "reviewer",
                        "agent": "pi",
                        "agent_status": "idle",
                        "workspace_id": "w1",
                        "tab_id": "w1:t1",
                        "focused": true,
                        "revision": 0
                    }
                }
            })
        )
        .unwrap();
        prompt_stream.flush().unwrap();
    });

    let prompted = run_cli(
        &socket_path,
        &[
            "agent",
            "prompt",
            "w1:p1",
            "review this",
            "--wait",
            "--until",
            "idle",
        ],
    );
    assert!(
        prompted.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&prompted.stderr)
    );
    let prompted: serde_json::Value = serde_json::from_slice(&prompted.stdout).unwrap();
    assert_eq!(prompted["result"]["agent"]["name"], "reviewer");

    server.join().unwrap();
    cleanup_test_base(&base);
}

#[test]
fn t3_lifecycle_and_prompts_use_the_adapter_without_a_pane() {
    let cases = [
        (
            vec![
                "agent",
                "start",
                "worker",
                "--kind",
                "t3",
                "--scratch",
                "--provider",
                "codex",
                "--model",
                "fixture",
            ],
            serde_json::json!({"action":"start","title":"worker","scratch":true,"modelSelection":{"instanceId":"codex","model":"fixture"}}),
        ),
        (
            vec![
                "agent",
                "stop",
                "t3:thread:opaque",
                "--client-request-id",
                "stop-1",
            ],
            serde_json::json!({"action":"stop","target":"t3:thread:opaque","clientRequestId":"stop-1"}),
        ),
        (
            vec![
                "agent",
                "resume",
                "t3:thread:opaque",
                "--client-request-id",
                "resume-1",
            ],
            serde_json::json!({"action":"resume","target":"t3:thread:opaque","clientRequestId":"resume-1"}),
        ),
        (
            vec![
                "agent",
                "prompt",
                "t3:thread:opaque",
                "literal $(echo text)",
                "--client-request-id",
                "prompt-1",
            ],
            serde_json::json!({"action":"prompt","target":"t3:thread:opaque","text":"literal $(echo text)","clientRequestId":"prompt-1"}),
        ),
        (
            vec![
                "agent",
                "read",
                "t3:thread:opaque",
                "--after-position",
                "42",
            ],
            serde_json::json!({"action":"read","target":"t3:thread:opaque","afterPosition":42}),
        ),
        (
            vec!["agent", "status", "t3:thread:opaque"],
            serde_json::json!({"action":"status","target":"t3:thread:opaque"}),
        ),
    ];
    for (args, expected) in cases {
        let base = unique_test_dir();
        fs::create_dir_all(&base).unwrap();
        let socket = base.join("herdr.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let server = thread::spawn(move || {
            let (mut stream, line) = accept_fake_cli_operation(&listener);
            let request: serde_json::Value = serde_json::from_str(&line).unwrap();
            assert_eq!(request["method"], "agent.external");
            assert_eq!(request["params"], expected);
            writeln!(stream, "{}", serde_json::json!({"id":request["id"],"result":{"type":"external_agent","data":{"receipt":"fixture-receipt"}}})).unwrap();
        });
        let result = run_cli(&socket, &args);
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let output: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
        assert_eq!(output["result"]["data"]["receipt"], "fixture-receipt");
        server.join().unwrap();
        cleanup_test_base(&base);
    }
}

#[test]
fn t3_rejects_unsupported_options_before_connecting_or_mutating() {
    let base = unique_test_dir();
    let socket = base.join("absent.sock");
    for args in [
        vec!["agent", "start", "worker", "--kind", "t3", "--scratch"],
        vec![
            "agent",
            "start",
            "worker",
            "--kind",
            "t3",
            "--scratch",
            "--project",
            "project",
            "--provider",
            "codex",
            "--model",
            "fixture",
        ],
        vec!["agent", "prompt", "t3:thread", "hello"],
        vec![
            "agent",
            "prompt",
            "t3:thread",
            "hello",
            "--client-request-id",
            "key",
            "--wait",
        ],
        vec!["agent", "focus", "t3:thread"],
    ] {
        let output = run_cli(&socket, &args);
        assert!(!output.status.success());
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(!error.contains("server_not_running"), "{error}");
        assert!(
            error.contains("T3") || error.contains("projectId"),
            "{error}"
        );
    }
}

#[test]
fn agent_list_negotiates_catalog_and_retains_old_server_fallback() {
    for supports_catalog in [false, true] {
        let base = unique_test_dir();
        fs::create_dir_all(&base).unwrap();
        let socket = base.join("herdr.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut line = String::new();
            BufReader::new(stream.try_clone().unwrap())
                .read_line(&mut line)
                .unwrap();
            let ping: serde_json::Value = serde_json::from_str(&line).unwrap();
            assert_eq!(ping["method"], "ping");
            writeln!(stream, "{}", serde_json::json!({"id":ping["id"],"result":{"type":"pong","version":"fixture","protocol":CURRENT_PROTOCOL,"capabilities":{"live_handoff":false,"agent_catalog":supports_catalog}}})).unwrap();
            stream.flush().unwrap();
            let (mut stream, line) = accept_fake_cli_operation(&listener);
            let request: serde_json::Value = serde_json::from_str(&line).unwrap();
            assert_eq!(
                request["method"],
                if supports_catalog {
                    "agent.catalog"
                } else {
                    "agent.list"
                }
            );
            let result = if supports_catalog {
                serde_json::json!({
                    "type": "agent_catalog",
                    "agents": [
                        {"id":"pane-1","backend":"terminal","name":"terminal worker",
                         "status":"idle","capabilities":["read","prompt","stop"],
                         "placement":{"terminal_id":"terminal-1","workspace_id":"workspace-1",
                                      "tab_id":"tab-1","pane_id":"pane-1"}},
                        {"id":"t3:thread-1","backend":"t3","name":"T3 worker",
                         "status":"working","capabilities":["read","prompt","stop","resume"]}
                    ],
                    "diagnostics": []
                })
            } else {
                serde_json::json!({"type":"agent_list","agents":[]})
            };
            writeln!(
                stream,
                "{}",
                serde_json::json!({"id":request["id"],"result":result})
            )
            .unwrap();
        });
        let result = run_cli(&socket, &["agent", "list"]);
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let output: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
        let result = &output["result"];
        if supports_catalog {
            assert_eq!(result["type"], "agent_catalog");
            assert_eq!(result["agents"].as_array().unwrap().len(), 2);
            assert_eq!(result["agents"][0]["placement"]["pane_id"], "pane-1");
            assert_eq!(result["agents"][1]["id"], "t3:thread-1");
            assert!(result["agents"][1].get("placement").is_none());
            assert_eq!(result["agents"][1]["status"], "working");
            assert_eq!(
                result["agents"][1]["capabilities"],
                serde_json::json!(["read", "prompt", "stop", "resume"])
            );
            assert_eq!(result["diagnostics"], serde_json::json!([]));
        } else {
            assert_eq!(
                result,
                &serde_json::json!({"type":"agent_list","agents":[]})
            );
        }
        server.join().unwrap();
        cleanup_test_base(&base);
    }
}
