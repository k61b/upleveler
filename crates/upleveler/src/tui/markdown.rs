//! Markdown → styled, width-wrapped terminal lines.

use super::theme;
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

pub fn width(s: &str) -> usize {
    UnicodeWidthStr::width(s)
}

/// Word-wraps styled spans to `width` columns, prefixing the first line with
/// `first` and the following ones with `rest`.
pub fn wrap(
    spans: &[Span<'static>],
    max: usize,
    first: &Span<'static>,
    rest: &Span<'static>,
) -> Vec<Line<'static>> {
    // Split into alternating word / whitespace pieces that keep their style.
    let mut pieces: Vec<(String, Style, bool)> = Vec::new();
    for span in spans {
        let mut current = String::new();
        let mut current_ws = false;
        for c in span.content.chars() {
            let ws = c.is_whitespace();
            if !current.is_empty() && ws != current_ws {
                pieces.push((std::mem::take(&mut current), span.style, current_ws));
            }
            current_ws = ws;
            current.push(if c == '\n' || c == '\t' { ' ' } else { c });
        }
        if !current.is_empty() {
            pieces.push((current, span.style, current_ws));
        }
    }

    let mut lines = Vec::new();
    let mut line: Vec<Span<'static>> = vec![first.clone()];
    let mut used = width(&first.content);
    let mut prefix_width = used;
    let mut empty = true;
    let new_line = |lines: &mut Vec<Line<'static>>, line: &mut Vec<Span<'static>>| {
        lines.push(Line::from(std::mem::replace(line, vec![rest.clone()])));
    };
    for (text, style, ws) in pieces {
        let w = width(&text);
        if ws {
            if !empty && used + w <= max {
                line.push(Span::styled(text, style));
                used += w;
            }
            continue;
        }
        if !empty && used + w > max {
            // Drop trailing whitespace before breaking.
            if line
                .last()
                .is_some_and(|s| s.content.trim().is_empty() && line.len() > 1)
            {
                line.pop();
            }
            new_line(&mut lines, &mut line);
            prefix_width = width(&rest.content);
            used = prefix_width;
        }
        if prefix_width + w > max {
            // A single word longer than the line: hard-split it.
            let mut chunk = String::new();
            for c in text.chars() {
                let cw = UnicodeWidthChar::width(c).unwrap_or(0);
                if used + cw > max && !chunk.is_empty() {
                    line.push(Span::styled(std::mem::take(&mut chunk), style));
                    new_line(&mut lines, &mut line);
                    prefix_width = width(&rest.content);
                    used = prefix_width;
                }
                chunk.push(c);
                used += cw;
            }
            line.push(Span::styled(chunk, style));
        } else {
            line.push(Span::styled(text, style));
            used += w;
        }
        empty = false;
    }
    lines.push(Line::from(line));
    lines
}

struct ListState {
    next: Option<u64>,
}

struct TableState {
    rows: Vec<Vec<String>>,
    row: Vec<String>,
    cell: String,
    header_rows: usize,
}

struct Writer {
    max: usize,
    out: Vec<Line<'static>>,
    spans: Vec<Span<'static>>,
    styles: Vec<Style>,
    lists: Vec<ListState>,
    item_marker: Option<String>,
    quote: usize,
    heading: Option<u8>,
    code_block: bool,
    table: Option<TableState>,
}

impl Writer {
    fn style(&self) -> Style {
        self.styles
            .iter()
            .fold(Style::default(), |acc, s| acc.patch(*s))
    }

    fn indent(&self) -> String {
        let mut s = "│ ".repeat(self.quote);
        s.push_str(&"  ".repeat(self.lists.len().saturating_sub(1)));
        s
    }

    fn blank(&mut self) {
        if self.out.last().is_some_and(|l| l.width() > 0) {
            self.out.push(Line::default());
        }
    }

    fn flush(&mut self) {
        if self.spans.is_empty() {
            return;
        }
        let indent = self.indent();
        let (first, rest) = match self.item_marker.take() {
            Some(marker) => {
                let pad = " ".repeat(width(&marker));
                (format!("{indent}{marker}"), format!("{indent}{pad}"))
            }
            None if !self.lists.is_empty() => {
                let pad = format!("{indent}  ");
                (pad.clone(), pad)
            }
            None => (indent.clone(), indent),
        };
        let prefix_style = if self.quote > 0 {
            theme::dim()
        } else {
            Style::default()
        };
        let spans = std::mem::take(&mut self.spans);
        let lines = wrap(
            &spans,
            self.max,
            &Span::styled(first, prefix_style),
            &Span::styled(rest, prefix_style),
        );
        self.out.extend(lines);
    }

    fn text(&mut self, t: &str) {
        if let Some(table) = &mut self.table {
            table.cell.push_str(t);
            return;
        }
        if self.code_block {
            for line in t.trim_end_matches('\n').split('\n') {
                let indent = self.indent();
                self.out.push(Line::from(vec![
                    Span::raw(format!("{indent}  ")),
                    Span::styled(line.to_string(), theme::code()),
                ]));
            }
            return;
        }
        let style = self.style();
        self.spans.push(Span::styled(t.to_string(), style));
    }

    fn table(&mut self, table: TableState) {
        let cols = table.rows.iter().map(Vec::len).max().unwrap_or(0);
        if cols == 0 {
            return;
        }
        let gap = 2;
        let mut widths: Vec<usize> = (0..cols)
            .map(|c| {
                table
                    .rows
                    .iter()
                    .filter_map(|r| r.get(c))
                    .map(|s| width(s))
                    .max()
                    .unwrap_or(0)
                    .max(1)
            })
            .collect();
        // Shrink the widest column until the table fits.
        while widths.iter().sum::<usize>() + gap * (cols - 1) > self.max {
            let (i, w) = widths
                .iter()
                .copied()
                .enumerate()
                .max_by_key(|&(_, w)| w)
                .unwrap_or((0, 0));
            if w <= 6 {
                break;
            }
            widths[i] = w - 1;
        }
        for (r, row) in table.rows.iter().enumerate() {
            let style = if r < table.header_rows {
                theme::accent_bold()
            } else {
                Style::default()
            };
            let cells: Vec<Vec<Line<'static>>> = (0..cols)
                .map(|c| {
                    let text = row.get(c).cloned().unwrap_or_default();
                    wrap(
                        &[Span::styled(text, style)],
                        widths[c],
                        &Span::raw(""),
                        &Span::raw(""),
                    )
                })
                .collect();
            let height = cells.iter().map(Vec::len).max().unwrap_or(1);
            for h in 0..height {
                let mut spans = vec![Span::raw(self.indent())];
                for (c, cell) in cells.iter().enumerate() {
                    let line = cell.get(h).cloned().unwrap_or_default();
                    let pad = widths[c].saturating_sub(line.width());
                    spans.extend(line.spans);
                    if c + 1 < cols {
                        spans.push(Span::raw(" ".repeat(pad + gap)));
                    }
                }
                self.out.push(Line::from(spans));
            }
            if r + 1 == table.header_rows {
                let total = widths.iter().sum::<usize>() + gap * (cols - 1);
                self.out
                    .push(Line::styled("─".repeat(total.min(self.max)), theme::dim()));
            }
        }
    }

    fn event(&mut self, event: Event) {
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(t) => self.text(&t),
            Event::Code(t) => {
                if let Some(table) = &mut self.table {
                    table.cell.push_str(&t);
                } else {
                    self.spans.push(Span::styled(t.to_string(), theme::code()));
                }
            }
            Event::SoftBreak => self.text(" "),
            Event::HardBreak => self.flush(),
            Event::Rule => {
                self.flush();
                self.blank();
                self.out
                    .push(Line::styled("─".repeat(self.max.min(60)), theme::dim()));
                self.out.push(Line::default());
            }
            Event::Html(t) | Event::InlineHtml(t) => self.text(&t),
            Event::TaskListMarker(done) => self.text(if done { "[x] " } else { "[ ] " }),
            _ => {}
        }
    }

    fn start(&mut self, tag: Tag) {
        match tag {
            Tag::Paragraph => {}
            Tag::Heading { level, .. } => {
                self.flush();
                self.blank();
                let level = level as u8;
                self.heading = Some(level);
                let style = if level <= 2 {
                    theme::accent_bold()
                } else {
                    theme::bold()
                };
                self.styles.push(style);
            }
            Tag::BlockQuote(_) => {
                self.flush();
                self.quote += 1;
            }
            Tag::CodeBlock(_) => {
                self.flush();
                self.code_block = true;
            }
            Tag::List(start) => {
                self.flush();
                self.lists.push(ListState { next: start });
            }
            Tag::Item => {
                self.flush();
                let marker = match self.lists.last_mut().and_then(|l| l.next.as_mut()) {
                    Some(n) => {
                        let m = format!("{n}. ");
                        *n += 1;
                        m
                    }
                    None => "• ".to_string(),
                };
                self.item_marker = Some(marker);
            }
            Tag::Emphasis => self
                .styles
                .push(Style::default().add_modifier(Modifier::ITALIC)),
            Tag::Strong => self
                .styles
                .push(Style::default().add_modifier(Modifier::BOLD)),
            Tag::Strikethrough => self
                .styles
                .push(Style::default().add_modifier(Modifier::CROSSED_OUT)),
            Tag::Link { .. } => self
                .styles
                .push(theme::accent().add_modifier(Modifier::UNDERLINED)),
            Tag::Table(_) => {
                self.flush();
                self.blank();
                self.table = Some(TableState {
                    rows: Vec::new(),
                    row: Vec::new(),
                    cell: String::new(),
                    header_rows: 0,
                });
            }
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => {
                self.flush();
                if self.lists.is_empty() {
                    self.out.push(Line::default());
                }
            }
            TagEnd::Heading(_) => {
                self.flush();
                self.styles.pop();
                self.heading = None;
                self.out.push(Line::default());
            }
            TagEnd::BlockQuote(_) => {
                self.flush();
                self.quote = self.quote.saturating_sub(1);
                self.blank();
            }
            TagEnd::CodeBlock => {
                self.code_block = false;
                self.out.push(Line::default());
            }
            TagEnd::List(_) => {
                self.flush();
                self.lists.pop();
                if self.lists.is_empty() {
                    self.blank();
                }
            }
            TagEnd::Item => self.flush(),
            TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough | TagEnd::Link => {
                self.styles.pop();
            }
            TagEnd::TableCell => {
                if let Some(t) = &mut self.table {
                    let cell = std::mem::take(&mut t.cell).trim().to_string();
                    t.row.push(cell);
                }
            }
            TagEnd::TableHead => {
                if let Some(t) = &mut self.table {
                    let row = std::mem::take(&mut t.row);
                    t.rows.push(row);
                    t.header_rows = t.rows.len();
                }
            }
            TagEnd::TableRow => {
                if let Some(t) = &mut self.table {
                    let row = std::mem::take(&mut t.row);
                    t.rows.push(row);
                }
            }
            TagEnd::Table => {
                if let Some(t) = self.table.take() {
                    self.table(t);
                    self.out.push(Line::default());
                }
            }
            _ => {}
        }
    }
}

/// Renders Markdown to lines no wider than `max` columns.
pub fn render(md: &str, max: u16) -> Vec<Line<'static>> {
    let mut w = Writer {
        max: (max as usize).max(20),
        out: Vec::new(),
        spans: Vec::new(),
        styles: Vec::new(),
        lists: Vec::new(),
        item_marker: None,
        quote: 0,
        heading: None,
        code_block: false,
        table: None,
    };
    let options =
        Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    for event in Parser::new_ext(md, options) {
        w.event(event);
    }
    w.flush();
    while w.out.last().is_some_and(|l| l.width() == 0) {
        w.out.pop();
    }
    w.out
}

/// Plain text of a line, for tests and width checks.
pub fn plain(line: &Line) -> String {
    line.spans.iter().map(|s| s.content.as_ref()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(md: &str, w: u16) -> Vec<String> {
        render(md, w).iter().map(plain).collect()
    }

    #[test]
    fn headings_lists_and_wrapping() {
        let out = text(
            "## Öne çıkanlar\n\n- **Incident** yönetimi: search index bozuldu, reindex job'ı çalıştırdım\n- kısa\n\n1. bir\n2. iki\n",
            30,
        );
        assert_eq!(out[0], "Öne çıkanlar");
        assert_eq!(out[1], "");
        assert_eq!(out[2], "• Incident yönetimi: search");
        assert!(out[3].starts_with("  index bozuldu,"), "{out:?}");
        assert!(out.contains(&"• kısa".to_string()));
        assert!(out.contains(&"1. bir".to_string()) && out.contains(&"2. iki".to_string()));
        assert!(render("x", 30).iter().all(|l| l.width() <= 30));
        for l in render(
            "- çok uzun bir madde ki satıra kesinlikle sığmayacak kadar uzun yazılmış",
            24,
        ) {
            assert!(l.width() <= 24, "{:?}", plain(&l));
        }
    }

    #[test]
    fn tables_align_with_emoji_widths() {
        let md =
            "| | Area | Entries |\n|---|---|---|\n| ✅ | Ownership | 2 |\n| ❌ | Mentoring | 0 |\n";
        let out = text(md, 60);
        assert_eq!(out[0], "    Area       Entries");
        assert!(out[1].starts_with("──"));
        assert_eq!(out[2], "✅  Ownership  2");
        assert_eq!(out[3], "❌  Mentoring  0");
    }

    #[test]
    fn narrow_tables_wrap_cells() {
        let md = "| Area | Expectation |\n|---|---|\n| Teknik | Birden fazla servisi etkileyen çözümler tasarlar |\n";
        for l in render(md, 30) {
            assert!(l.width() <= 30, "{:?}", plain(&l));
        }
    }

    #[test]
    fn long_words_are_split() {
        let out = render("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", 20);
        assert!(out.len() >= 3);
        assert!(out.iter().all(|l| l.width() <= 20));
    }
}
