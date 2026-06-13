fn main() -> std::io::Result<()> {
    eventline::init_sync();
    eventline::enable_console_output(false);
    eventline::enable_file_output_jsonl("/tmp/eventline-example.jsonl")?;

    eventline::info!("user login", user_id = 42, method = "oauth");
    eventline::warn!("quota nearly exhausted", user_id = 42, remaining = 3);
    eventline::flush()?;

    Ok(())
}
