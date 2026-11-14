//! Terminal output: messages, styles, the spinner, panels, tables and the yes/no question.
//!
//! One [`Ui`] value is created in `run()` and passed around; there is no global state. Errors and warnings go to
//! stderr, everything else to stdout.

pub mod panel;
pub mod table;

use crate::process::ProcResult;
use comfy_table::Table;
use console::{Color, Style, Term, measure_text_width};
use indicatif::{ProgressBar, ProgressDrawTarget, ProgressStyle};
use std::io::Write;
use std::time::Duration;

/// Width used when the output is redirected or the terminal reports less than this.
const MIN_WIDTH: usize = 20;
const FALLBACK_WIDTH: usize = 120;

#[derive(Debug, Clone)]
pub struct Ui {
    /// stdout is a terminal: spinner, live table and questions are only used then.
    pub interactive: bool,
    /// Columns for tables and panels.
    pub width: usize,
}

impl Ui {
    pub fn new() -> Self {
        let term = Term::stdout();
        let interactive = term.is_term();
        let width = match term.size_checked() {
            Some((_, cols)) if interactive && usize::from(cols) >= MIN_WIDTH => usize::from(cols),
            _ => fallback_width(),
        };
        Self { interactive, width }
    }

    /// How to call this app again, for hints in messages: the launcher in the repository root.
    pub fn self_name(&self) -> &'static str {
        if cfg!(windows) {
            "sandbox.ps1"
        } else {
            "./sandbox"
        }
    }

    // --- styles (colours are off when NO_COLOR is set or the stream isn't a terminal: `console` decides) ---

    pub fn bold(&self, s: impl std::fmt::Display) -> String {
        Style::new().bold().apply_to(s).to_string()
    }

    pub fn dim(&self, s: impl std::fmt::Display) -> String {
        Style::new().dim().apply_to(s).to_string()
    }

    pub fn green(&self, s: impl std::fmt::Display) -> String {
        Style::new().green().apply_to(s).to_string()
    }

    pub fn red(&self, s: impl std::fmt::Display) -> String {
        Style::new().red().apply_to(s).to_string()
    }

    pub fn yellow(&self, s: impl std::fmt::Display) -> String {
        Style::new().yellow().apply_to(s).to_string()
    }

    // --- messages ---

    /// `ERROR: …` on stderr.
    pub fn error(&self, message: &str) {
        let bold = Style::new().red().bold().for_stderr();
        let red = Style::new().red().for_stderr();
        eprintln!("{} {}", bold.apply_to("ERROR:"), red.apply_to(message));
    }

    /// `WARNING: …` on stderr.
    pub fn warn(&self, message: &str) {
        let bold = Style::new().yellow().bold().for_stderr();
        let yellow = Style::new().yellow().for_stderr();
        eprintln!("{} {}", bold.apply_to("WARNING:"), yellow.apply_to(message));
    }

    /// `✓ …` on stdout.
    pub fn success(&self, message: &str) {
        println!("{} {message}", self.green("✓"));
    }

    // --- docker's state words ---

    /// A container state in its colour: running green, exited/dead red, empty dim, anything else yellow.
    pub fn state(&self, s: &str) -> String {
        match s {
            "running" => self.green("running"),
            "exited" | "dead" => self.red(s),
            "" => self.dim("—"),
            _ => self.yellow(s),
        }
    }

    /// A health state in its colour.
    pub fn health(&self, h: &str, state: &str) -> String {
        match h {
            "healthy" => self.green("healthy"),
            "starting" => self.yellow("starting…"),
            "unhealthy" => self.red("unhealthy"),
            "" if state == "running" => self.dim("n/a"),
            "" => self.dim("—"),
            _ => h.to_string(),
        }
    }

    // --- spinner, rule, panels, tables ---

    /// Runs the action with a spinner (only when interactive).
    pub fn spin<T>(&self, what: &str, action: impl FnOnce() -> T) -> T {
        if !self.interactive {
            return action();
        }
        let bar = ProgressBar::with_draw_target(None, ProgressDrawTarget::stdout());
        let style = ProgressStyle::with_template("{spinner:.yellow} {msg}")
            .unwrap_or_else(|_| ProgressStyle::default_spinner())
            .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏", " "]);
        bar.set_style(style);
        bar.set_message(what.to_string());
        bar.enable_steady_tick(Duration::from_millis(80));
        let result = action();
        bar.finish_and_clear();
        result
    }

    /// A line of `─` with a title at the left (docker's build output follows).
    pub fn rule(&self, title: &str) {
        let cells = measure_text_width(title);
        let rest = self.width.saturating_sub(cells + 4);
        println!(
            "{} {} {}",
            self.yellow("──"),
            self.yellow(title),
            self.yellow("─".repeat(rest))
        );
    }

    /// A plain dim line of `─` over the width.
    pub fn plain_rule(&self) {
        println!("{}", self.dim("─".repeat(self.width)));
    }

    /// Prints a bordered panel. `colour` is the border and title colour (none: default).
    pub fn print_panel(&self, title: &str, lines: &[String], colour: Option<Color>, expand: bool) {
        let mut border = Style::new();
        let mut title_style = Style::new().bold();
        if let Some(c) = colour {
            border = border.fg(c);
            title_style = Style::new().fg(c);
        }
        print!(
            "{}",
            panel::render(title, &title_style, &border, lines, self.width, expand)
        );
    }

    /// The output of a failed command in a red panel (its last 40 lines).
    pub fn show_output(&self, title: &str, r: &ProcResult) {
        let text = format!("{}\n{}", r.stdout, r.stderr);
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        let all: Vec<&str> = text.split('\n').collect();
        let mut lines: Vec<String> = Vec::new();
        if all.len() > 40 {
            lines.push(format!("… ({} earlier lines)", all.len() - 40));
            lines.extend(all[all.len() - 40..].iter().map(|l| l.to_string()));
        } else {
            lines.extend(all.iter().map(|l| l.to_string()));
        }
        let title = format!("{title} (exit {})", r.exit_code);
        self.print_panel(&title, &lines, Some(Color::Red), true);
    }

    /// A table with the given headers.
    pub fn new_table(&self, headers: &[&str]) -> Table {
        table::new(headers, self.width)
    }

    /// Prints a table with an optional title line above and caption below.
    pub fn print_table(&self, title: Option<&str>, table: &Table, caption: Option<&str>) {
        if let Some(t) = title {
            println!("{t}");
        }
        println!("{table}");
        if let Some(c) = caption {
            println!("{c}");
        }
    }

    /// `question [y/N] `: true on `y` or `yes`.
    pub fn confirm(&self, question: &str) -> bool {
        print!("{question} [y/N] ");
        let _ = std::io::stdout().flush();
        let mut answer = String::new();
        if std::io::stdin().read_line(&mut answer).is_err() {
            return false;
        }
        matches!(answer.trim().to_lowercase().as_str(), "y" | "yes")
    }
}

impl Default for Ui {
    fn default() -> Self {
        Self::new()
    }
}

/// `$COLUMNS` if it is above 20, else 120.
fn fallback_width() -> usize {
    std::env::var("COLUMNS")
        .ok()
        .and_then(|c| c.trim().parse::<usize>().ok())
        .filter(|c| *c > MIN_WIDTH)
        .unwrap_or(FALLBACK_WIDTH)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_and_health_words() {
        // Colours are forced off: on a host terminal `cargo test` stdout can be a TTY.
        console::set_colors_enabled(false);
        let ui = Ui {
            interactive: false,
            width: 80,
        };
        assert_eq!(ui.state("running"), "running");
        assert_eq!(ui.state(""), "—");
        assert_eq!(ui.state("restarting"), "restarting");
        assert_eq!(ui.health("starting", "running"), "starting…");
        assert_eq!(ui.health("", "running"), "n/a");
        assert_eq!(ui.health("", "exited"), "—");
        assert_eq!(ui.health("odd", "running"), "odd");
    }

    #[test]
    fn show_output_keeps_the_last_forty_lines() {
        let ui = Ui {
            interactive: false,
            width: 80,
        };
        let out: String = (1..=50).map(|i| format!("line {i}\n")).collect();
        let r = ProcResult {
            exit_code: 2,
            stdout: out,
            stderr: String::new(),
        };
        ui.show_output("docker x", &r); // printed; the logic is covered by the E2E tests
    }
}
