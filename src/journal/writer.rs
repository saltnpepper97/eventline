use std::io::Write;
use super::Journal;
use crate::render::canonical::{render_scope_header, render_event, RenderConfig};
use super::{Record, RecordKind};

/// Writer for rendering journals to different output sinks.
///
/// Uses the canonical "Narrative Structured" format for all output destinations.
/// Ensures file logs, stdout, and replay all look identical.
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
#[derive(Debug, Clone)]
pub struct JournalWriter {
    config: RenderConfig,
}

impl Default for JournalWriter {
    fn default() -> Self {
        Self::new()
    }
}

impl JournalWriter {
    /// Create a new journal writer with default canonical settings.
    pub fn new() -> Self {
        Self {
            config: RenderConfig::default(),
        }
    }

    /// Create a writer with color disabled.
    pub fn no_color() -> Self {
        Self {
            config: RenderConfig::no_color(),
        }
    }

    /// Create a writer without timestamps.
    pub fn no_timestamps() -> Self {
        Self {
            config: RenderConfig::no_timestamps(),
        }
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
        self.config.bullet = bullet.into();
        self
    }

    /// Enable or disable color in output.
    pub fn with_color(mut self, color: bool) -> Self {
        self.config.color = color;
        self
    }

    /// Enable or disable timestamps in scope headers.
    pub fn with_timestamps(mut self, timestamps: bool) -> Self {
        self.config.timestamps = timestamps;
        self
    }

    /// Internal method to prepare scope metadata for rendering.
    /// 
    /// Returns events grouped by scope for efficient lookup during rendering.
    fn prepare_metadata<'a>(
        &self,
        journal: &'a Journal,
    ) -> std::collections::HashMap<crate::journal::ScopeId, Vec<&'a Record>> {
        use std::collections::HashMap;

        let mut events_by_scope: HashMap<crate::journal::ScopeId, Vec<&Record>> = HashMap::new();

        for record in journal.records() {
            if let Some(scope) = record.scope {
                if matches!(record.kind, RecordKind::Event { .. }) {
                    events_by_scope.entry(scope).or_default().push(record);
                }
            }
        }

        events_by_scope
    }

    /// Internal method to render journal content directly to writers using canonical format.
    /// 
    /// This is the single source of truth for rendering logic.
    /// Zero-allocation streaming for optimal performance.
    fn render_to_writers(
        &self,
        writers: &mut [&mut dyn Write],
        journal: &Journal,
    ) -> std::io::Result<()> {
        let events_by_scope = self.prepare_metadata(journal);

        for scope in journal.scopes() {
            // Render scope header using canonical format
            let scope_header = render_scope_header(journal, scope, &self.config);

            // Write scope header to all writers
            for writer in writers.iter_mut() {
                writeln!(writer, "{}", scope_header.header)?;
            }

            // Write scope events to all writers
            if let Some(events) = events_by_scope.get(&scope.id) {
                for event in events {
                    if let Some(rendered) = render_event(event, &self.config, 1) {
                        // Write main event line
                        for writer in writers.iter_mut() {
                            writeln!(writer, "{}", rendered.main)?;
                        }

                        // Write detail line if present (arrow rule: only if it adds information)
                        if let Some(detail) = rendered.detail {
                            for writer in writers.iter_mut() {
                                writeln!(writer, "{}", detail)?;
                            }
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
    /// Uses canonical format for all destinations, ensuring consistency.
    ///
    /// # Example
    /// ```
    /// use eventline::journal::{Journal, JournalWriter};
    /// use std::io;
    /// 
    /// let journal = Journal::new();
    /// let mut file = std::fs::File::create("output.log")?;
    /// 
    /// // Write to both stdout and file - both will look identical
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
    use crate::journal::outcome::Outcome;

    #[test]
    fn test_journal_writer_custom_bullet() {
        let writer = JournalWriter::new().with_bullet("-");
        assert_eq!(writer.config.bullet, "-");
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
        
        // Verify canonical format
        let output = String::from_utf8(buf1).unwrap();
        assert!(output.contains("→"));
        assert!(output.contains("(id="));
        assert!(output.contains("ms)"));
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

    #[test]
    fn test_no_color_output() -> std::io::Result<()> {
        let mut journal = Journal::new();
        let scope = journal.enter_scope_unnamed(None);
        journal.record(Some(scope), "test");
        journal.exit_scope(scope, Outcome::Success);

        let mut buf = Vec::new();
        JournalWriter::no_color().write_to(&mut buf, &journal)?;

        let output = String::from_utf8(buf).unwrap();
        // Should not contain ANSI escape codes
        assert!(!output.contains("\x1b["));
        Ok(())
    }
}
