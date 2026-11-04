import unittest
from unittest import mock

from scripts import hc_resume as h


def term(argv, app=None, labels=None, fg_name="node", pid=42, cwd="/w"):
    return {
        "address": "t1",
        "name": "lane",
        "argv": argv,
        "status": {"app": app} if app else None,
        "labels": labels or {},
        "foreground": {"pid": pid, "cwd": cwd, "name": fg_name},
        "cwd": cwd,
    }


@mock.patch.object(h, "agent_dir", return_value=None)
class PlanTest(unittest.TestCase):
    def test_pi_session_id_argv_resumes_that_session_and_keeps_model(self, _):
        rec = h.plan(term(["pi", "--session-id", "abc", "--model", "m", "do the task"], app="pi"), ["pi", "", ""])
        self.assertEqual(rec["argv"], ["pi", "--session", "abc", "--model", "m"])
        self.assertEqual(rec["source"], "argv")

    def test_pi_inside_sh_wrapper_and_label_fallback(self, _):
        wrapped = h.plan(term(["sh", "-c", "exec pi --session /s/x.jsonl --thinking high"], app="pi"), [])
        self.assertEqual(wrapped["argv"], ["pi", "--session", "/s/x.jsonl", "--thinking", "high"])
        labelled = h.plan(term(["/bin/zsh", "-l"], app="pi", labels={"harbor.harness-session-id": "lid"}), [])
        self.assertEqual((labelled["argv"], labelled["source"]), (["pi", "--session", "lid"], "label"))

    def test_pii_profile_is_detected_from_its_agent_dir(self, agent_dir):
        agent_dir.return_value = "/Users/x/.pi/pii"
        with mock.patch.object(h, "pi_newest", return_value="/p.jsonl") as newest:
            rec = h.plan(term(["/bin/zsh", "-l"], app="pi"), [])
        self.assertEqual(rec["argv"], ["pii", "--session", "/p.jsonl"])
        newest.assert_called_once_with("pii", "/w", "/Users/x/.pi/pii")

    def test_claude_uses_pid_session_and_running_flags_without_prompt(self, _):
        with mock.patch.object(h, "claude_session", return_value="cid"), mock.patch.object(h, "claude_model", return_value="x"):
            rec = h.plan(term(["/bin/zsh"], app="claude-code"), ["claude", "--dangerously-skip-permissions", "--model", "haiku", "hello"])
        self.assertEqual(rec["argv"], ["claude", "--resume", "cid", "--dangerously-skip-permissions", "--model", "haiku"])

    def test_codex_names_recorded_provider_and_model_and_never_drops_no_daemon(self, _):
        with mock.patch.object(h, "codex_session", return_value="xid"), mock.patch.object(h, "codex_meta", return_value=("openai", "r/m")):
            rec = h.plan(term(["codex"], app="codex"), ["codex", "--profile", "p", "--yolo-ish", "task"])
        self.assertEqual(rec["argv"], ["codex", "resume", "xid", "--no-daemon", "-c", 'model_provider="openai"', "-m", "r/m"])

    def test_plain_programs_and_unknown_sessions_are_not_resumed(self, _):
        self.assertIsNone(h.plan(term(["python3", "app.py"]), []))
        with mock.patch.object(h, "codex_session", return_value=None):
            self.assertIsNone(h.plan(term(["codex"], app="codex"), [])["argv"])


if __name__ == "__main__":
    unittest.main()
