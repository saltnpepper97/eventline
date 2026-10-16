use eventline::{FileFormat, FileSetup, LogLevel, LogPolicy, RunHeader, Setup, WriteMode};
use std::fs;
use std::future::Future;
use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::task::{Context, Poll, Waker};

static RUNTIME: Mutex<()> = Mutex::new(());
static NEXT_DIR: AtomicUsize = AtomicUsize::new(0);

struct TestRuntime {
    dir: PathBuf,
    _lock: MutexGuard<'static, ()>,
}

impl TestRuntime {
    fn new() -> Self {
        let lock = RUNTIME.lock().unwrap();
        eventline::disable_all_output();
        eventline::set_writer_mode(WriteMode::BestEffort).unwrap();
        eventline::set_file_level(LogLevel::Debug);
        eventline::set_file_format(FileFormat::Text);
        eventline::runtime::clear();
        let dir = std::env::temp_dir().join(format!(
            "eventline-reliability-{}-{}",
            std::process::id(),
            NEXT_DIR.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&dir).unwrap();
        Self { dir, _lock: lock }
    }
}

impl Drop for TestRuntime {
    fn drop(&mut self) {
        eventline::disable_all_output();
        fs::remove_dir_all(&self.dir).unwrap();
    }
}

fn run_setup(
    file: FileSetup,
    level: Option<LogLevel>,
    file_level: Option<LogLevel>,
) -> io::Result<()> {
    let mut future = Box::pin(eventline::setup(Setup {
        verbose: false,
        level,
        console_level: None,
        file_level,
        file: Some(file),
        journal_retention: None,
    }));
    match future
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
    {
        Poll::Ready(result) => result,
        Poll::Pending => panic!("setup unexpectedly suspended"),
    }
}

#[test]
fn every_file_initializer_returns_open_errors_and_retains_diagnostics() {
    let env = TestRuntime::new();
    assert!(eventline::enable_file_output(&env.dir).is_err());
    assert!(eventline::enable_file_output_jsonl(&env.dir).is_err());
    assert!(eventline::enable_file_output_rotating(&env.dir, LogPolicy::new(1, 1), None).is_err());
    assert!(env.dir.is_dir());
    let diagnostics = eventline::writer_diagnostics();
    assert_eq!(diagnostics.failed_operations, 3);
    assert_eq!(diagnostics.dropped_records, 0);
    assert!(
        diagnostics
            .last_error
            .unwrap()
            .contains(env.dir.to_str().unwrap())
    );
    assert_eq!(eventline::get_log_level(), LogLevel::Off);
}

#[test]
fn failed_file_replacement_preserves_the_active_sink_and_level() {
    let env = TestRuntime::new();
    let path = env.dir.join("active.log");
    eventline::set_file_level(LogLevel::Error);
    eventline::enable_file_output(&path).unwrap();
    eventline::error!("before failed replacement");
    let error = eventline::enable_file_output(env.dir.join("missing/new.log")).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::NotFound);
    assert!(error.raw_os_error().is_some());
    eventline::error!("after failed replacement");
    eventline::info!("filtered event");
    eventline::flush().unwrap();
    let output = fs::read_to_string(path).unwrap();
    assert!(output.contains("before failed replacement"));
    assert!(output.contains("after failed replacement"));
    assert!(!output.contains("filtered event"));
    assert_eq!(eventline::get_log_level(), LogLevel::Error);
    assert_eq!(eventline::records().len(), 3);
    assert_eq!(eventline::writer_diagnostics().failed_operations, 1);
}

#[test]
fn the_same_failed_path_can_be_retried_and_success_does_not_erase_errors() {
    let env = TestRuntime::new();
    let path = env.dir.join("missing/retry.log");
    assert!(eventline::enable_file_output(&path).is_err());
    let error = eventline::last_writer_error();
    fs::create_dir(env.dir.join("missing")).unwrap();
    eventline::enable_file_output(&path).unwrap();
    eventline::info!("recovered");
    eventline::flush().unwrap();
    assert!(fs::read_to_string(path).unwrap().contains("recovered"));
    assert_eq!(eventline::last_writer_error(), error);
    assert_eq!(eventline::writer_diagnostics().failed_operations, 1);
    eventline::runtime::clear();
    assert_eq!(
        eventline::writer_diagnostics(),
        eventline::WriterDiagnostics::default()
    );
}

#[test]
fn setup_propagates_file_initialization_errors_for_every_format() {
    let env = TestRuntime::new();
    for file in [
        FileSetup::Plain {
            path: env.dir.clone(),
        },
        FileSetup::Jsonl {
            path: env.dir.clone(),
        },
        FileSetup::Rotating {
            path: env.dir.clone(),
            policy: LogPolicy::default(),
            header: None,
        },
    ] {
        assert!(run_setup(file, None, None).is_err());
    }
    assert_eq!(eventline::writer_diagnostics().failed_operations, 3);
}

#[test]
fn setup_preserves_explicit_file_levels_across_all_file_initializers() {
    let env = TestRuntime::new();
    for (name, file) in [
        (
            "plain.log",
            FileSetup::Plain {
                path: env.dir.join("plain.log"),
            },
        ),
        (
            "jsonl.log",
            FileSetup::Jsonl {
                path: env.dir.join("jsonl.log"),
            },
        ),
        (
            "rotating.log",
            FileSetup::Rotating {
                path: env.dir.join("rotating.log"),
                policy: LogPolicy::default(),
                header: None,
            },
        ),
    ] {
        run_setup(file, Some(LogLevel::Debug), Some(LogLevel::Warning)).unwrap();
        eventline::debug!("debug hidden");
        eventline::info!("info hidden");
        eventline::warn!("warning visible");
        eventline::error!("error visible");
        eventline::flush().unwrap();
        let output = fs::read_to_string(env.dir.join(name)).unwrap();
        assert!(!output.contains("debug hidden"));
        assert!(!output.contains("info hidden"));
        assert!(output.contains("warning visible"));
        assert!(output.contains("error visible"));
        assert_eq!(eventline::get_log_level(), LogLevel::Warning);
    }
    assert_eq!(eventline::records().len(), 12);
}

#[test]
fn setup_shared_fallback_level_is_not_replaced_by_debug() {
    let env = TestRuntime::new();
    let path = env.dir.join("fallback.log");
    run_setup(
        FileSetup::Plain { path: path.clone() },
        Some(LogLevel::Error),
        None,
    )
    .unwrap();
    eventline::warn!("warning hidden");
    eventline::error!("error visible");
    eventline::flush().unwrap();
    let output = fs::read_to_string(path).unwrap();
    assert!(!output.contains("warning hidden"));
    assert!(output.contains("error visible"));
}

#[cfg(unix)]
#[test]
fn setup_can_replace_an_unlinked_sink_without_losing_its_requested_level() {
    let env = TestRuntime::new();
    let old_dir = env.dir.join("old");
    fs::create_dir(&old_dir).unwrap();
    let old_path = old_dir.join("removed.log");
    eventline::enable_file_output(&old_path).unwrap();
    fs::remove_file(old_path).unwrap();
    fs::remove_dir(old_dir).unwrap();

    let path = env.dir.join("replacement.log");
    run_setup(
        FileSetup::Plain { path: path.clone() },
        Some(LogLevel::Debug),
        Some(LogLevel::Error),
    )
    .unwrap();
    eventline::warn!("warning hidden");
    eventline::error!("error visible");
    eventline::flush().unwrap();
    let output = fs::read_to_string(path).unwrap();
    assert!(!output.contains("warning hidden"));
    assert!(output.contains("error visible"));
    assert_eq!(
        eventline::writer_diagnostics(),
        eventline::WriterDiagnostics::default()
    );
}

#[test]
fn failed_setup_does_not_partially_replace_sink_levels() {
    let env = TestRuntime::new();
    let path = env.dir.join("active.log");
    run_setup(
        FileSetup::Plain { path: path.clone() },
        None,
        Some(LogLevel::Error),
    )
    .unwrap();
    assert!(
        run_setup(
            FileSetup::Plain {
                path: env.dir.clone()
            },
            Some(LogLevel::Debug),
            Some(LogLevel::Warning)
        )
        .is_err()
    );
    eventline::warn!("still hidden");
    eventline::error!("still visible");
    eventline::flush().unwrap();
    let output = fs::read_to_string(path).unwrap();
    assert!(!output.contains("still hidden"));
    assert!(output.contains("still visible"));
    assert_eq!(eventline::get_log_level(), LogLevel::Error);
}

#[cfg(target_os = "linux")]
#[test]
fn actual_disk_write_failures_are_counted_and_returned_by_flush() {
    let env = TestRuntime::new();
    eventline::enable_file_output("/dev/full").unwrap();
    eventline::info!("cannot write first");
    eventline::info!("cannot write second");
    assert_eq!(
        eventline::flush().unwrap_err().kind(),
        io::ErrorKind::StorageFull
    );
    let diagnostics = eventline::writer_diagnostics();
    assert_eq!(diagnostics.failed_operations, 2);
    assert_eq!(diagnostics.dropped_records, 0);
    assert!(diagnostics.last_error.unwrap().contains("writing record"));
    assert_eq!(eventline::records().len(), 2);
    eventline::enable_file_output(env.dir.join("recovered.log")).unwrap();
    eventline::info!("recovered");
    eventline::flush().unwrap();
    assert_eq!(eventline::writer_diagnostics().failed_operations, 2);
}

#[cfg(target_os = "linux")]
#[test]
fn header_write_failures_are_returned_and_do_not_replace_healthy_output() {
    let env = TestRuntime::new();
    let path = env.dir.join("healthy.log");
    eventline::enable_file_output(&path).unwrap();
    // Zero disables rotation, ensuring the device itself is never renamed.
    let error = eventline::enable_file_output_rotating(
        "/dev/full",
        LogPolicy::new(0, 0),
        Some(RunHeader::new("failed header")),
    )
    .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::StorageFull);
    assert!(error.raw_os_error().is_some());
    eventline::info!("still healthy");
    eventline::flush().unwrap();
    assert!(fs::read_to_string(path).unwrap().contains("still healthy"));
    assert_eq!(eventline::writer_diagnostics().failed_operations, 1);
}

#[test]
fn initialization_returns_rotation_errors_without_discarding_the_active_file() {
    let env = TestRuntime::new();
    let path = env.dir.join("rotate.log");
    fs::write(&path, "previous run").unwrap();
    fs::create_dir(env.dir.join("rotate.log.1")).unwrap();
    assert!(eventline::enable_file_output_rotating(&path, LogPolicy::new(1, 1), None).is_err());
    assert_eq!(fs::read_to_string(path).unwrap(), "previous run");
    assert_eq!(eventline::writer_diagnostics().failed_operations, 1);
}

#[test]
fn background_rotation_errors_reach_diagnostics_and_flush() {
    let env = TestRuntime::new();
    let path = env.dir.join("rotate.log");
    eventline::enable_file_output_rotating(&path, LogPolicy::new(1, 1), None).unwrap();
    eventline::info!("first record");
    eventline::flush().unwrap();
    fs::create_dir(env.dir.join("rotate.log.1")).unwrap();
    eventline::info!("rotation fails");
    assert!(eventline::flush().is_err());
    let output = fs::read_to_string(path).unwrap();
    assert!(output.contains("first record"));
    assert!(!output.contains("rotation fails"));
    assert_eq!(eventline::writer_diagnostics().failed_operations, 1);
    assert_eq!(eventline::records().len(), 2);
}

#[test]
fn zero_limit_keeps_queued_records_before_a_new_header_and_disables_rotation() {
    let env = TestRuntime::new();
    let path = env.dir.join("ordered.log");
    eventline::set_writer_mode(WriteMode::Blocking).unwrap();
    eventline::enable_file_output(&path).unwrap();
    for index in 0..1000 {
        eventline::info!("queued-{index}");
    }
    eventline::enable_file_output_rotating(
        &path,
        LogPolicy::new(0, 1),
        Some(RunHeader::new("next session")),
    )
    .unwrap();
    eventline::info!("after header");
    eventline::flush().unwrap();
    let output = fs::read_to_string(&path).unwrap();
    assert_eq!(output.matches("queued-").count(), 1000);
    assert!(output.find("queued-999").unwrap() < output.find("next session").unwrap());
    assert!(output.find("next session").unwrap() < output.find("after header").unwrap());
    assert!(!env.dir.join("ordered.log.1").exists());
    assert_eq!(
        eventline::writer_diagnostics(),
        eventline::WriterDiagnostics::default()
    );
}

#[test]
fn oversized_header_is_not_rotated_away_by_a_second_file_open() {
    let env = TestRuntime::new();
    let path = env.dir.join("header.log");
    fs::write(&path, "previous run").unwrap();
    eventline::enable_file_output_rotating(
        &path,
        LogPolicy::new(1, 1),
        Some(RunHeader::new("large header")),
    )
    .unwrap();
    eventline::flush().unwrap();
    assert!(fs::read_to_string(path).unwrap().contains("large header"));
    assert_eq!(
        fs::read_to_string(env.dir.join("header.log.1")).unwrap(),
        "previous run"
    );
}
