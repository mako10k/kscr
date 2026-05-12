use std::process::Command;
use std::{fs, path::Path};

fn emit_rerun_for_dir(path: &Path) {
    println!("cargo:rerun-if-changed={}", path.display());
    let Ok(entries) = fs::read_dir(path) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        println!("cargo:rerun-if-changed={}", path.display());
        if path.is_dir() {
            emit_rerun_for_dir(&path);
        }
    }
}

fn main() {
    // Capture git SHA at build time
    let git_sha = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .and_then(|output| {
            if output.status.success() {
                String::from_utf8(output.stdout).ok()
            } else {
                None
            }
        })
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    println!("cargo:rustc-env=KSCR_GIT_SHA={}", git_sha);

    // Rerun if git HEAD changes
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/refs/heads");

    // The installed binary embeds stdlib/ and `kscr install-stdlib` extracts that copy.
    // Rebuild whenever any stdlib file changes so the extracted stdlib stays in sync.
    emit_rerun_for_dir(Path::new("stdlib"));
}
