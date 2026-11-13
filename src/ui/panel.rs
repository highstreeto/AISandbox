//! A bordered panel with a title in the top line.

use console::{Style, measure_text_width};

/// ```text
/// ╭─ Title ───────╮
/// │ first line    │
/// │ second        │
/// ╰───────────────╯
/// ```
///
/// With `expand` the panel is at least `width` columns wide. Long lines are not wrapped (the terminal does that).
pub fn render(
    title: &str,
    title_style: &Style,
    border: &Style,
    lines: &[String],
    width: usize,
    expand: bool,
) -> String {
    let content = lines
        .iter()
        .map(|l| measure_text_width(l))
        .max()
        .unwrap_or(0);
    let title_cells = if title.is_empty() {
        0
    } else {
        measure_text_width(title) + 2
    };
    let mut inner = (content + 2).max(title_cells + 2);
    if expand {
        inner = inner.max(width.saturating_sub(2));
    }

    let mut out = String::new();
    let fill = inner - 1 - title_cells;
    out.push_str(&border.apply_to("╭─").to_string());
    if !title.is_empty() {
        out.push_str(&title_style.apply_to(format!(" {title} ")).to_string());
    }
    out.push_str(
        &border
            .apply_to(format!("{}╮", "─".repeat(fill)))
            .to_string(),
    );
    out.push('\n');
    for line in lines {
        let pad = inner - 2 - measure_text_width(line);
        out.push_str(&border.apply_to("│").to_string());
        out.push(' ');
        out.push_str(line);
        out.push_str(&" ".repeat(pad + 1));
        out.push_str(&border.apply_to("│").to_string());
        out.push('\n');
    }
    out.push_str(
        &border
            .apply_to(format!("╰{}╯", "─".repeat(inner)))
            .to_string(),
    );
    out.push('\n');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fits_the_content_and_the_title() {
        let plain = Style::new();
        let lines = vec!["abc".to_string(), "abcdef".to_string()];
        let text = render("T", &plain, &plain, &lines, 80, false);
        assert_eq!(text, "╭─ T ────╮\n│ abc    │\n│ abcdef │\n╰────────╯\n");
    }

    #[test]
    fn expands_to_the_width() {
        let plain = Style::new();
        let text = render("Title", &plain, &plain, &["x".to_string()], 20, true);
        let widths: Vec<usize> = text.lines().map(measure_text_width).collect();
        assert_eq!(widths, vec![20, 20, 20]);
    }

    #[test]
    fn a_long_title_widens_the_panel() {
        let plain = Style::new();
        let text = render(
            "A long title",
            &plain,
            &plain,
            &["x".to_string()],
            80,
            false,
        );
        let widths: Vec<usize> = text.lines().map(measure_text_width).collect();
        assert!(widths.iter().all(|w| *w == widths[0]), "{text}");
    }
}
