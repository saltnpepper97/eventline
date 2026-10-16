fn main() -> std::io::Result<()> {
    let paths: Vec<_> = std::env::args_os().skip(1).collect();
    if paths.is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "usage: cargo run --example replay_file -- <oldest.jsonl> [newer.jsonl ...]",
        ));
    }
    let saved = eventline::replay::load_jsonl_files(&paths)?;
    for (index, run) in saved.runs.iter().enumerate() {
        println!(
            "Run {}: {}",
            index + 1,
            run.run_id.as_deref().unwrap_or("legacy")
        );
        for header in &run.headers {
            println!("{header}");
        }
        println!("{}", eventline::replay::render_scope_tree(&run.scopes));
        println!(
            "{} records, {} failures, {} unfinished scopes",
            run.records.len(),
            eventline::replay::failed_scopes(&run.records, &run.scopes).len(),
            run.unfinished_scopes().len()
        );
    }
    for issue in &saved.issues {
        eprintln!("recovery issue: {issue:?}");
    }
    Ok(())
}
