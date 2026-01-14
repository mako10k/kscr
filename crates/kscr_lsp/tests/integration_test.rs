//! Integration tests for the LSP server

use std::process::{Command, Stdio};

#[test]
fn test_lsp_server_binary_exists() {
    // Build the LSP server
    let status = Command::new("cargo")
        .args(&["build", "--release"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .status()
        .expect("Failed to build LSP server");

    assert!(status.success(), "LSP server build failed");

    // Check that the binary exists
    let binary_path = format!(
        "{}/target/release/kscr-lsp{}",
        env!("CARGO_MANIFEST_DIR"),
        if cfg!(windows) { ".exe" } else { "" }
    );
    assert!(
        std::path::Path::new(&binary_path).exists(),
        "LSP server binary not found at {}",
        binary_path
    );
}

#[test]
fn test_lsp_server_can_start() {
    // Try to start the LSP server (it will wait for input on stdin)
    let mut child = Command::new("cargo")
        .args(&["run", "--release"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Failed to start LSP server");

    // Give it a moment to start
    std::thread::sleep(std::time::Duration::from_millis(500));

    // Check if the process is still running (LSP server should be waiting for input)
    match child.try_wait() {
        Ok(Some(status)) => {
            panic!(
                "LSP server exited unexpectedly with status: {:?}",
                status
            );
        }
        Ok(None) => {
            // Process is still running, which is expected
        }
        Err(e) => {
            panic!("Failed to check LSP server status: {}", e);
        }
    }

    // Kill the process
    child.kill().expect("Failed to kill LSP server");
    let output = child.wait_with_output().expect("Failed to wait for LSP server");

    // Check that stderr doesn't contain panic or error messages
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("panic") && !stderr.contains("error:"),
        "LSP server produced unexpected errors: {}",
        stderr
    );
}
