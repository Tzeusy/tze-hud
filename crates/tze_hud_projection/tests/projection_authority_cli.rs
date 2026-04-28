use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};

struct AuthorityCli {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<std::process::ChildStdout>,
}

impl AuthorityCli {
    fn spawn() -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_tze_hud_projection_authority"))
            .arg("--stdio")
            .arg("--caller-identity")
            .arg("cli-test-caller")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("projection authority CLI starts");
        let stdin = child.stdin.take().expect("stdin is piped");
        let stdout = BufReader::new(child.stdout.take().expect("stdout is piped"));
        Self {
            child,
            stdin,
            stdout,
        }
    }

    fn send(&mut self, request: Value) -> Value {
        writeln!(self.stdin, "{request}").expect("request line writes");
        self.stdin.flush().expect("stdin flushes");
        let mut line = String::new();
        self.stdout
            .read_line(&mut line)
            .expect("response line reads");
        serde_json::from_str(&line).expect("response is JSON")
    }
}

impl Drop for AuthorityCli {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn stdio_surface_delegates_to_projection_authority_and_emits_audit_records() {
    let mut cli = AuthorityCli::spawn();

    let attach = cli.send(json!({
        "operation": "attach",
        "projection_id": "cli-projection",
        "request_id": "req-attach",
        "client_timestamp_wall_us": 1,
        "provider_kind": "codex",
        "display_name": "Codex CLI Test",
        "content_classification": "private",
        "idempotency_key": "cli-projection-once"
    }));
    assert_eq!(attach["response"]["accepted"], true);
    assert_eq!(attach["response"]["projection_id"], "cli-projection");
    let owner_token = attach["response"]["owner_token"]
        .as_str()
        .expect("attach returns owner token")
        .to_string();
    assert_eq!(attach["audit_records"][0]["operation"], "attach");
    assert_eq!(
        attach["audit_records"][0]["caller_identity"],
        "cli-test-caller"
    );
    assert!(
        !serde_json::to_string(&attach["audit_records"])
            .unwrap()
            .contains(&owner_token)
    );

    let published = cli.send(json!({
        "operation": "publish_output",
        "projection_id": "cli-projection",
        "request_id": "req-output",
        "client_timestamp_wall_us": 2,
        "owner_token": owner_token,
        "output_text": "private projected CLI transcript",
        "output_kind": "assistant",
        "content_classification": "private",
        "logical_unit_id": "cli-turn-1"
    }));
    assert_eq!(published["response"]["accepted"], true);
    assert_eq!(published["response"]["owner_token"], Value::Null);
    assert!(
        !serde_json::to_string(&published["audit_records"])
            .unwrap()
            .contains("private projected CLI transcript")
    );

    let denied = cli.send(json!({
        "operation": "get_pending_input",
        "projection_id": "cli-projection",
        "request_id": "req-denied",
        "client_timestamp_wall_us": 3,
        "owner_token": "wrong-token"
    }));
    assert_eq!(denied["response"]["accepted"], false);
    assert_eq!(denied["response"]["error_code"], "PROJECTION_UNAUTHORIZED");
    assert!(
        denied["response"]["pending_input"]
            .as_array()
            .is_none_or(Vec::is_empty)
    );
    assert_eq!(denied["audit_records"][0]["category"], "auth_denied");
}
