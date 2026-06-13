fn main() {
    eventline::init_sync();
    eventline::enable_console_output(true);

    eventline::info!("starting app", version = env!("CARGO_PKG_VERSION"));

    eventline::scope!("startup", success = "ready", failure = "failed", {
        eventline::debug!("loading configuration", path = "config.toml");
        eventline::info!("configuration loaded");
    });

    let _ = eventline::flush();
}
