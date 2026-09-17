use serde_json::{json, Value};
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Stdio},
};
fn invoke(args: &[&str], request: &Value) -> std::process::Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_eleph-codegen"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let payload = request.to_string();
    let writer = std::thread::spawn(move || {
        let _ = stdin.write_all(payload.as_bytes());
    });
    let output = child.wait_with_output().unwrap();
    writer.join().unwrap();
    output
}
fn request() -> Value {
    json!({"elephentity":1,"irVersion":"1.1","schema":{"opaque":"the orchestrator must not inspect this"}})
}
fn response() -> Value {
    json!({"elephentity":1,"irVersion":"1.1","headerStyle":"php","extensions":["php"],"files":[{"path":"Post.php","body":"namespace Fixture;\n\nfinal class Post\n{\n}\n"}],"errors":[]})
}
#[cfg(unix)]
fn builder(root: &Path, name: &str, response: &Value, output_first: bool) {
    use std::os::unix::fs::PermissionsExt;
    let path = root.join(name);
    let print = format!(
        "printf '%s' '{}'\n",
        response.to_string().replace('\'', "'\\''")
    );
    let script = if output_first {
        format!("#!/bin/sh\n{print}cat >/dev/null\n")
    } else {
        format!("#!/bin/sh\ncat >/dev/null\n{print}")
    };
    fs::write(&path, script).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}
fn config(root: &Path, targets: Value) {
    fs::write(
        root.join("eleph.json"),
        json!({"spec":"spec","targets":targets}).to_string(),
    )
    .unwrap();
}
#[test]
fn rejects_unsupported_compiler_version() {
    let output = invoke(
        &["generate"],
        &json!({"elephentity":1,"irVersion":"99","schema":{}}),
    );
    assert_eq!(output.status.code(), Some(2));
}
#[test]
#[cfg(unix)]
fn signs_checks_and_preserves_nested_targets() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let path = root.to_str().unwrap();
    builder(root, "builder", &response(), false);
    config(
        root,
        json!({"php":{"output":"generated","builder":"./builder"},"nested":{"output":"generated/nested","builder":"./builder"}}),
    );
    let output = invoke(&["generate", "--project", path], &request());
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let generated = root.join("generated/Post.php");
    let signed = fs::read_to_string(&generated).unwrap();
    assert!(
        signed.contains("sha256:cff42c4dd731d4940868532b99381523560408f85b8d8d103321e24bc02cde0f")
    );
    assert!(
        invoke(&["generate", "--project", path, "--check"], &request())
            .status
            .success()
    );
    fs::write(&generated, signed.replace("class Post", "class Edited")).unwrap();
    let before = fs::read(&generated).unwrap();
    let output = invoke(&["generate", "--project", path, "--check"], &request());
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stdout).contains("Hand-edited"));
    assert_eq!(before, fs::read(&generated).unwrap());
    fs::write(root.join("generated/stale.php"), "old").unwrap();
    fs::write(root.join("generated/notes.txt"), "keep").unwrap();
    fs::write(root.join("generated/nested/extra.php"), "reserved").unwrap();
    assert!(invoke(
        &["generate", "--project", path, "--targets", "php"],
        &request()
    )
    .status
    .success());
    assert!(!root.join("generated/stale.php").exists());
    assert!(root.join("generated/notes.txt").exists());
    assert!(root.join("generated/nested/extra.php").exists());
}
#[test]
#[cfg(unix)]
fn pools_errors_before_writing_any_target() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    builder(root, "good", &response(), false);
    builder(
        root,
        "bad-a",
        &json!({"elephentity":1,"irVersion":"1.1","headerStyle":"php","errors":["first failure"]}),
        false,
    );
    builder(
        root,
        "bad-b",
        &json!({"elephentity":1,"irVersion":"1.1","headerStyle":"php","errors":["second failure"]}),
        false,
    );
    config(
        root,
        json!({"good":{"output":"good-out","builder":"./good"},"a":{"output":"a-out","builder":"./bad-a"},"b":{"output":"b-out","builder":"./bad-b"}}),
    );
    let output = invoke(
        &["generate", "--project", root.to_str().unwrap()],
        &request(),
    );
    assert_eq!(output.status.code(), Some(1));
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(text.contains("first failure") && text.contains("second failure"));
    assert!(!root.join("good-out").exists());
}
#[test]
#[cfg(unix)]
fn rejects_paths_outside_output_and_forwards_large_payloads_without_deadlock() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    config(
        root,
        json!({"php":{"output":"generated","builder":"./builder"}}),
    );
    let mut bad = response();
    bad["files"][0]["path"] = json!("../escape.php");
    builder(root, "builder", &bad, false);
    assert!(!invoke(
        &["generate", "--project", root.to_str().unwrap()],
        &request()
    )
    .status
    .success());
    assert!(!root.join("escape.php").exists());
    let mut large = response();
    large["files"][0]["body"] = json!("x".repeat(256 * 1024));
    builder(root, "builder", &large, true);
    let mut input = request();
    input["schema"]["large"] = json!("y".repeat(256 * 1024));
    assert!(
        invoke(&["generate", "--project", root.to_str().unwrap()], &input)
            .status
            .success()
    );
    assert!(fs::metadata(root.join("generated/Post.php")).unwrap().len() > 256 * 1024);
}
