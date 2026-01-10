use std::io::Write;
use super::Journal;
use super::utils::millis_to_local;

use crate::event_kind::EventKind;
use crate::id::ScopeId;
use crate::outcome::Outcome;
use crate::record::{Record, RecordKind};

/// Writer for rendering journals to different output sinks.
///
/// Separates rendering policy from journal data. Supports streaming
/// output to multiple destinations simultaneously.
///
/// # Example
/// ```
/// use eventline::journal::{Journal, JournalWriter};
/// use std::io;
/// 
/// let journal = Journal::new();
/// let writer = JournalWriter::new();
/// 
/// // Write to stdout
/// writer.write_to(&mut io::stdout(), &journal)?;
/// 
/// // Write to file
/// let mut file = std::fs::File::create("output.log")?;
/// writer.write_to(&mut file, &journal)?;
/// # Ok::<(), std::io::Error>(())
/// ```
#[derive(Debug, Default, Clone)]
pub struct JournalWriter {
    bullet: Option<String>,
}

impl JournalWriter {
    /// Create a new journal writer with default settings.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set a custom bullet character for event listings.
    ///
    /// # Example
    /// ```
    /// use eventline::journal::JournalWriter;
    /// 
    /// let writer = JournalWriter::new().with_bullet("-");
    /// ```
    pub fn with_bullet(mut self, bullet: impl Into<String>) -> Self {
        self.bullet = Some(bullet.into());
        self
    }

    /// Internal method to prepare scope metadata for rendering.
    /// 
    /// Returns (events_by_scope, exits) hashmaps for efficient lookup during rendering.
    /// Events and exits are kept separate to avoid coupling and make future record
    /// kinds easier to handle.
    fn prepare_metadata<'a>(
        &self,
        journal: &'a Journal,
    ) -> (
        std::collections::HashMap<ScopeId, Vec<&'a Record>>,
        std::collections::HashMap<ScopeId, &'a Record>,
    ) {
        use std::collections::HashMap;

        let mut events_by_scope: HashMap<ScopeId, Vec<&Record>> = HashMap::new();
        let mut exits: HashMap<ScopeId, &Record> = HashMap::new();

        for record in journal.records() {
            if let Some(scope) = record.scope {
                match &record.kind {
                    RecordKind::Event { .. } => {
                        events_by_scope.entry(scope).or_default().push(record);
                    }
                    RecordKind::ScopeExit { .. } => {
                        exits.insert(scope, record);
                    }
                }
            }
        }

        (events_by_scope, exits)
    }

    /// Internal method to render journal content directly to writers.
    /// 
    /// This is the single source of truth for rendering logic.
    /// Zero-allocation streaming for optimal performance.
    fn render_to_writers(
        &self,
        writers: &mut [&mut dyn Write],
        journal: &Journal,
    ) -> std::io::Result<()> {
        let bullet = self.bullet.as_deref()
            .unwrap_or(if cfg!(windows) { "*" } else { "•" });

        let (events_by_scope, exits) = self.prepare_metadata(journal);

        for scope in journal.scopes() {
            let exit = exits.get(&scope.id);

            let outcome = exit
                .and_then(|r| {
                    if let RecordKind::ScopeExit { outcome, .. } = r.kind {
                        Some(outcome)
                    } else {
                        None
                    }
                })
                .unwrap_or(Outcome::Aborted);

            let duration_ms = exit
                .and_then(|r| {
                    if let RecordKind::ScopeExit { exited_at, .. } = r.kind {
                        Some(exited_at.saturating_sub(scope.entered_at))
                    } else {
                        None
                    }
                })
                .unwrap_or(0);

            let duration_s = duration_ms as f64 / 1000.0;
            let ts = millis_to_local(scope.entered_at);

            // Write scope header to all writers
            for writer in writers.iter_mut() {
                writeln!(
                    writer,
                    "[{}] Scope {} ({:?}) [{:.3}s]",
                    ts,
                    scope.id.0,
                    outcome,
                    duration_s
                )?;
            }

            // Write scope events to all writers
            if let Some(events) = events_by_scope.get(&scope.id) {
                for event in events {
                    // Safe unwrap: events_by_scope only contains Event records
                    if let RecordKind::Event { kind, message } = &event.kind {
                        let prefix = match kind {
                            EventKind::Info => "",
                            EventKind::Warning => "warning: ",
                            EventKind::Error => "error: ",
                            EventKind::Debug => "debug: ",
                        };

                        for writer in writers.iter_mut() {
                            writeln!(
                                writer,
                                "  {} {}{}",
                                bullet,
                                prefix,
                                message
                            )?;
                        }
                    }
                }
            }
        }

        // Flush all writers
        for writer in writers.iter_mut() {
            writer.flush()?;
        }

        Ok(())
    }

    /// Write the journal to a single output sink.
    ///
    /// # Example
    /// ```
    /// use eventline::journal::{Journal, JournalWriter};
    /// use std::io;
    /// 
    /// let journal = Journal::new();
    /// JournalWriter::new().write_to(&mut io::stdout(), &journal)?;
    /// # Ok::<(), std::io::Error>(())
    /// ```
    pub fn write_to<W: Write>(&self, writer: &mut W, journal: &Journal) -> std::io::Result<()> {
        self.render_to_writers(&mut [writer], journal)
    }

    /// Write the journal to multiple output sinks simultaneously.
    ///
    /// Useful for dual output: terminal and file logging at once.
    /// Accepts any combination of writer types via trait objects.
    ///
    /// # Example
    /// ```
    /// use eventline::journal::{Journal, JournalWriter};
    /// use std::io;
    /// 
    /// let journal = Journal::new();
    /// let mut file = std::fs::File::create("output.log")?;
    /// 
    /// // Write to both stdout and file
    /// JournalWriter::new().write_to_all(
    ///     &mut [&mut io::stdout() as &mut dyn std::io::Write, &mut file as &mut dyn std::io::Write],
    ///     &journal
    /// )?;
    /// # Ok::<(), std::io::Error>(())
    /// ```
    pub fn write_to_all(
        &self,
        writers: &mut [&mut dyn Write],
        journal: &Journal,
    ) -> std::io::Result<()> {
        self.render_to_writers(writers, journal)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_journal_writer_custom_bullet() {
        let writer = JournalWriter::new().with_bullet("-");
        assert_eq!(writer.bullet, Some("-".to_string()));
    }

    #[test]
    fn test_journal_is_cloneable() {
        let mut journal = Journal::new();
        journal.enter_scope_unnamed(None);
        
        let _cloned = journal.clone();
        // Journal can be cheaply cloned without output policy
    }

    #[test]
    fn test_write_to_multiple_sinks() -> std::io::Result<()> {
        let mut journal = Journal::new();
        let scope = journal.enter_scope_unnamed(None);
        journal.record(Some(scope), "test event");
        journal.exit_scope(scope, Outcome::Success);

        let mut buf1 = Vec::new();
        let mut buf2 = Vec::new();

        JournalWriter::new().write_to_all(
            &mut [&mut buf1 as &mut dyn Write, &mut buf2 as &mut dyn Write],
            &journal
        )?;

        assert_eq!(buf1, buf2);
        assert!(!buf1.is_empty());
        Ok(())
    }

    #[test]
    fn test_write_to_mixed_types() -> std::io::Result<()> {
        use std::io::Cursor;
        
        let mut journal = Journal::new();
        let scope = journal.enter_scope_unnamed(None);
        journal.record(Some(scope), "mixed types test");
        journal.exit_scope(scope, Outcome::Success);

        let mut vec_writer = Vec::new();
        let mut cursor_writer = Cursor::new(Vec::new());

        // This works because we use &mut dyn Write
        JournalWriter::new().write_to_all(
            &mut [&mut vec_writer as &mut dyn Write, &mut cursor_writer as &mut dyn Write],
            &journal
        )?;

        assert_eq!(vec_writer, cursor_writer.into_inner());
        Ok(())
    }
}
