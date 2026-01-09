pub fn unsafe_used(tag: &str) {
    if std::env::var_os("KSCR_DEBUG_UNSAFE").is_some() {
        eprintln!("kscr(debug): unsafe used: {tag}");
    }
}
