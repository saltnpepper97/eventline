/// A run header is a single decorated line written as raw bytes at the top
/// of a log session, before eventline starts appending structured records.
///
/// Because it bypasses the normal record pipeline it is always visible in the
/// file regardless of the configured log level.
///
/// # Example output
///
/// With PID:
/// ```text
/// ==================== my-daemon run start (pid=18432) ====================
/// ```
///
/// Without PID:
/// ```text
/// ========================= my-daemon run start =========================
/// ```
#[derive(Debug, Clone)]
pub struct RunHeader {
    /// Label shown in the centre of the header line.
    pub title: String,
    /// Append `(pid=N)` to the label when `true`.
    pub show_pid: bool,
    /// Total target width of the rendered line in characters.
    /// The `=` padding fills the remaining space.  Defaults to 72.
    pub width: usize,
}

impl RunHeader {
    /// Create a header that includes the current process ID.
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title:    title.into(),
            show_pid: true,
            width:    72,
        }
    }

    /// Create a header without a PID annotation.
    pub fn without_pid(title: impl Into<String>) -> Self {
        Self {
            title:    title.into(),
            show_pid: false,
            width:    72,
        }
    }

    /// Set the total rendered width (number of characters per line).
    pub fn with_width(mut self, width: usize) -> Self {
        self.width = width;
        self
    }

    /// Render the header to an owned `String`.
    pub fn render(&self) -> String {
        let inner = if self.show_pid {
            format!(" {} (pid={}) ", self.title, std::process::id())
        } else {
            format!(" {} ", self.title)
        };

        // Always leave at least a minimal border even if the title is long.
        let padding = if inner.len() + 4 < self.width {
            self.width - inner.len()
        } else {
            4
        };
        let left  = padding / 2;
        let right = padding - left;

        format!("{}{}{}", "=".repeat(left), inner, "=".repeat(right))
    }
}
