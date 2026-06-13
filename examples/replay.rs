fn main() {
    eventline::init_sync();
    eventline::disable_all_output();

    eventline::scope!("request", success = "served", failure = "failed", {
        eventline::info!("handling request", request_id = 7);
    });

    let records = eventline::records();
    let scopes = eventline::scopes();

    println!("{}", eventline::replay::render_scope_tree(&scopes));
    println!(
        "failed scopes: {}",
        eventline::replay::failed_scopes(&records, &scopes).len()
    );
}
