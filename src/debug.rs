use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};

static SEEN: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

pub fn unsafe_used(tag: &str) {
    if std::env::var_os("KSCR_DEBUG_UNSAFE").is_none() {
        return;
    }

    let seen = SEEN.get_or_init(|| Mutex::new(HashSet::new()));
    let mut seen = seen.lock().unwrap();
    if seen.insert(tag.to_string()) {
        eprintln!("kscr(debug): unsafe used: {tag}");
    }
}
