fn main() {
    match umanager_launcher::dispatch() {
        Some(Ok(())) => {}
        result => {
            eprintln!(
                "{}",
                result.and_then(Result::err).unwrap_or_else(|| {
                    "usage: umanager-antigravity-launcher --launch-antigravity <application ID>"
                        .into()
                })
            );
            std::process::exit(1);
        }
    }
}
