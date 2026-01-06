use std::env;

fn main() {
    if let Err(e) = kscr::cli::run(env::args()) {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
