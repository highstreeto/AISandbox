//! Tables: `comfy-table` with the rounded style (`│` between columns).

use comfy_table::{Cell, CellAlignment, ContentArrangement, Table, TableStyle};
use comfy_table::{ContentLineStyle, LineStyle};

/// ```text
/// ╭──────┬───────╮
/// │ Host │ Group │
/// ├──────┼───────┤
/// │ a    │ b     │
/// ╰──────┴───────╯
/// ```
const ROUNDED: TableStyle = TableStyle::new()
    .top_border(LineStyle::new('╭', '─', '┬', '╮'))
    .header_lines(ContentLineStyle::new('│', '│', '│'))
    .header_separator(LineStyle::new('├', '─', '┼', '┤'))
    .content_lines(ContentLineStyle::new('│', '│', '│'))
    .bottom_border(LineStyle::new('╰', '─', '┴', '╯'));

/// A table with the given column headers, laid out for `width` columns.
pub fn new(headers: &[&str], width: usize) -> Table {
    let mut table = Table::new();
    table.load_style(ROUNDED);
    table.set_content_arrangement(ContentArrangement::Dynamic);
    table.set_width(u16::try_from(width).unwrap_or(u16::MAX));
    table.set_header(headers.iter().map(|h| Cell::new(*h)).collect::<Vec<_>>());
    table
}

/// Right-aligns a column (numbers).
pub fn align_right(table: &mut Table, column: usize) {
    if let Some(c) = table.column_mut(column) {
        c.set_cell_alignment(CellAlignment::Right);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draws_a_rounded_table_with_vertical_bars() {
        let mut t = new(&["Host", "Result"], 80);
        t.add_row(["a.example.com", "allowed"]);
        let text = t.to_string();
        let lines: Vec<&str> = text.lines().collect();
        assert!(
            lines[0].starts_with('╭') && lines[0].contains('┬') && lines[0].ends_with('╮'),
            "{text}"
        );
        assert!(
            lines[1].starts_with("│ Host") && lines[1].matches('│').count() == 3,
            "{text}"
        );
        assert!(
            lines[2].starts_with('├') && lines[2].contains('┼'),
            "{text}"
        );
        assert!(
            lines[3].contains("a.example.com") && lines[3].contains("│ allowed"),
            "{text}"
        );
        assert!(
            lines[4].starts_with('╰') && lines[4].ends_with('╯'),
            "{text}"
        );
        assert_eq!(lines.len(), 5, "no row separators: {text}");
    }
}
