//! Cells that are printed into the terminal's scrollback, above the live area.

use super::markdown::{self, wrap};
use super::theme;
use crate::analyze::GapSummary;
use crate::ladder::Ladder;
use crate::session::Status;
use crate::store::Entry;
use ratatui::style::Style;
use ratatui::text::{Line, Span};

pub type Lines = Vec<Line<'static>>;

fn wrapped(
    text: &str,
    width: u16,
    first: Span<'static>,
    rest: Span<'static>,
    style: Style,
) -> Lines {
    let mut out = Vec::new();
    for (i, para) in text.lines().enumerate() {
        let lead = if i == 0 { first.clone() } else { rest.clone() };
        out.extend(wrap(
            &[Span::styled(para.to_string(), style)],
            width as usize,
            &lead,
            &rest,
        ));
    }
    if out.is_empty() {
        out.push(Line::from(first));
    }
    out
}

pub fn user(text: &str, width: u16) -> Lines {
    let mut out = wrapped(
        text,
        width,
        Span::styled("› ", theme::accent_bold()),
        Span::raw("  "),
        theme::dim(),
    );
    out.push(Line::default());
    out
}

pub fn info(text: &str, width: u16) -> Lines {
    let mut out = wrapped(text, width, Span::raw("  "), Span::raw("  "), theme::dim());
    out.push(Line::default());
    out
}

pub fn success(text: &str, width: u16) -> Lines {
    let mut out = wrapped(
        text,
        width,
        Span::styled("✓ ", theme::good()),
        Span::raw("  "),
        Style::default(),
    );
    out.push(Line::default());
    out
}

pub fn error(text: &str, width: u16) -> Lines {
    let mut out = wrapped(
        text,
        width,
        Span::styled("✗ ", theme::bad()),
        Span::raw("  "),
        theme::bad(),
    );
    out.push(Line::default());
    out
}

/// A model answer or report, rendered from Markdown.
pub fn answer(md: &str, width: u16) -> Lines {
    let mut out = Vec::new();
    for (i, line) in markdown::render(md, width.saturating_sub(2))
        .into_iter()
        .enumerate()
    {
        let lead = if i == 0 {
            Span::styled("● ", theme::accent())
        } else {
            Span::raw("  ")
        };
        let mut spans = vec![lead];
        spans.extend(line.spans);
        out.push(Line::from(spans));
    }
    out.push(Line::default());
    out
}

/// A horizontal bar of `cells` cells split into strong / partial / none.
pub fn readiness_bar(summary: &GapSummary, cells: usize) -> Vec<Span<'static>> {
    let total = summary.rows.len().max(1);
    let strong = summary.count("strong") * cells / total;
    let partial = (summary.count("strong") + summary.count("partial")) * cells / total - strong;
    let rest = cells.saturating_sub(strong + partial);
    vec![
        Span::styled("█".repeat(strong), theme::good()),
        Span::styled("█".repeat(partial), theme::warn()),
        Span::styled("░".repeat(rest), theme::dim()),
    ]
}

pub fn welcome(status: &Status, width: u16) -> Lines {
    let w = (width as usize).clamp(30, 72);
    let inner = w - 4;
    let border = theme::accent();
    let title = format!(" ▲ upleveler v{} ", env!("CARGO_PKG_VERSION"));
    let mut out: Lines = vec![Line::from(vec![
        Span::styled("╭─", border),
        Span::styled(title.clone(), theme::accent_bold()),
        Span::styled(
            format!(
                "{}╮",
                "─".repeat(w.saturating_sub(3 + markdown::width(&title)))
            ),
            border,
        ),
    ])];
    let row = |spans: Vec<Span<'static>>| {
        let spans = fit(spans, inner);
        let used: usize = spans.iter().map(|s| markdown::width(&s.content)).sum();
        let mut line = vec![Span::styled("│ ", border)];
        line.extend(spans);
        line.push(Span::raw(" ".repeat(inner.saturating_sub(used))));
        line.push(Span::styled(" │", border));
        Line::from(line)
    };
    out.push(row(vec![]));

    match (&status.ladder_levels, &status.target) {
        (None, _) => out.push(row(vec![
            Span::styled("No career ladder yet · ", theme::dim()),
            Span::styled("/ladder import @file", theme::accent()),
        ])),
        (Some(_), None) => out.push(row(vec![
            Span::styled("Pick your levels · ", theme::dim()),
            Span::styled("/levels", theme::accent()),
        ])),
        (Some(_), Some(target)) => {
            let current = status.current.clone().unwrap_or_else(|| "?".into());
            let levels = format!("{current} → {target}  ");
            let mut spans = vec![Span::styled(levels.clone(), theme::bold())];
            match &status.latest_gap {
                Some(g) if g.target == *target && !g.rows.is_empty() => {
                    let label = format!("  {}/{} strong", g.count("strong"), g.rows.len());
                    let room =
                        inner.saturating_sub(markdown::width(&levels) + markdown::width(&label));
                    spans.extend(readiness_bar(g, room.clamp(4, 16)));
                    spans.push(Span::styled(label, theme::dim()));
                }
                _ => spans.push(Span::styled(
                    "run /gap to see where you stand",
                    theme::dim(),
                )),
            }
            out.push(row(spans));
        }
    }
    let streak = if status.streak > 0 {
        format!(" · 🔥 {}-day streak", status.streak)
    } else {
        String::new()
    };
    out.push(row(vec![Span::styled(
        format!("{} logs{streak} · {} today", status.entries, status.today),
        theme::dim(),
    )]));
    let place = if status.local { "local" } else { "remote" };
    out.push(row(vec![Span::styled(
        format!("{} · {place}", status.model),
        theme::dim(),
    )]));
    out.push(row(vec![]));
    out.push(row(vec![Span::raw(
        "Type what you did to log it, or ask anything.",
    )]));
    out.push(row(vec![
        Span::styled("/", theme::accent()),
        Span::styled(" commands · ", theme::dim()),
        Span::styled("?", theme::accent()),
        Span::styled(" shortcuts · ", theme::dim()),
        Span::styled("@", theme::accent()),
        Span::styled(" files", theme::dim()),
    ]));
    out.push(Line::from(Span::styled(
        format!("╰{}╯", "─".repeat(w - 2)),
        border,
    )));
    out.push(Line::default());
    out
}

fn icon(rating: &str) -> &'static str {
    match rating {
        "strong" => "●",
        "partial" => "◐",
        "none" => "○",
        _ => "?",
    }
}

/// Compact gap result: overview, priorities, colored coverage list.
pub fn gap(summary: &GapSummary, path: &std::path::Path, width: u16) -> Lines {
    let w = width as usize;
    let mut out = vec![Line::from(vec![
        Span::styled("● ", theme::accent()),
        Span::styled(
            format!(
                "Gap analysis {} → {}",
                summary.current.as_deref().unwrap_or("?"),
                summary.target
            ),
            theme::accent_bold(),
        ),
    ])];
    let counts = format!(
        "  {} strong · {} partial · {} missing",
        summary.count("strong"),
        summary.count("partial"),
        summary.count("none")
    );
    let room = w.saturating_sub(2 + markdown::width(&counts));
    let mut bar = vec![Span::raw("  ")];
    bar.extend(readiness_bar(summary, room.clamp(4, 24)));
    bar.push(Span::styled(counts, theme::dim()));
    out.push(Line::from(fit(bar, w)));
    out.push(Line::default());
    if !summary.overview.is_empty() {
        out.extend(wrapped(
            &summary.overview,
            width,
            Span::raw("  "),
            Span::raw("  "),
            Style::default(),
        ));
        out.push(Line::default());
    }
    let area_w = summary
        .rows
        .iter()
        .map(|r| markdown::width(&r.area))
        .max()
        .unwrap_or(4)
        .min(18);
    for r in &summary.rows {
        let area = truncate(&r.area, area_w);
        let pad = " ".repeat(area_w.saturating_sub(markdown::width(&area)));
        let count = format!("{:>3}", r.count);
        let lead = Span::raw(format!("  {} {area}{pad}  ", icon(&r.rating)));
        let lead_w = markdown::width(&lead.content);
        let text_w = w.saturating_sub(lead_w + 5).max(10);
        let mut lines = wrap(
            &[Span::raw(r.text.clone())],
            text_w,
            &Span::raw(""),
            &Span::raw(""),
        );
        let first = lines.remove(0);
        let used = first.width();
        let mut spans = vec![Span::styled(lead.content.clone(), theme::rating(&r.rating))];
        spans.extend(first.spans);
        spans.push(Span::raw(" ".repeat(text_w.saturating_sub(used))));
        spans.push(Span::styled(count, theme::dim()));
        out.push(Line::from(spans));
        for l in lines {
            let mut spans = vec![Span::raw(" ".repeat(lead_w))];
            spans.extend(l.spans);
            out.push(Line::from(spans));
        }
    }
    if !summary.priorities.is_empty() {
        out.push(Line::default());
        out.push(Line::styled("  Priorities", theme::bold()));
        for (i, p) in summary.priorities.iter().enumerate() {
            out.extend(wrapped(
                p,
                width,
                Span::styled(format!("  {}. ", i + 1), theme::accent()),
                Span::raw("     "),
                Style::default(),
            ));
        }
    }
    out.push(Line::default());
    out.extend(wrapped(
        &format!("Full report: {} · /reports to read it", path.display()),
        width,
        Span::raw("  "),
        Span::raw("  "),
        theme::dim(),
    ));
    out.push(Line::default());
    out
}

pub fn ladder(ladder: &Ladder, current: Option<&str>, target: Option<&str>, width: u16) -> Lines {
    let mut out = Vec::new();
    for l in &ladder.levels {
        let mark = if current == Some(l.id.as_str()) {
            Span::styled("  ← you are here", theme::good())
        } else if target == Some(l.id.as_str()) {
            Span::styled("  ← target", theme::accent())
        } else {
            Span::raw("")
        };
        out.push(Line::from(vec![
            Span::styled(format!("  {} ", l.id), theme::accent_bold()),
            Span::styled(l.title.clone(), theme::bold()),
            mark,
        ]));
        let mut area = "";
        for e in &l.expectations {
            if e.area != area {
                area = &e.area;
                out.push(Line::styled(format!("    {area}"), theme::dim()));
            }
            out.extend(wrapped(
                &e.text,
                width,
                Span::raw("      • "),
                Span::raw("        "),
                Style::default(),
            ));
        }
        out.push(Line::default());
    }
    out
}

pub fn entries(entries: &[&Entry], width: u16) -> Lines {
    let mut out = Vec::new();
    for e in entries {
        let tags = if e.tags.is_empty() {
            String::new()
        } else {
            format!(" [{}]", e.tags.join(", "))
        };
        let lead = Span::styled(format!("  {} ", e.date), theme::accent());
        let mut spans = vec![Span::raw(e.text.replace('\n', " / "))];
        if !tags.is_empty() {
            spans.push(Span::styled(tags, theme::dim()));
        }
        out.extend(wrap(
            &spans,
            width as usize,
            &lead,
            &Span::raw("             "),
        ));
    }
    out.push(Line::default());
    out
}

/// Cuts spans so their total width is at most `max`, ending with "…" if cut.
pub fn fit(spans: Vec<Span<'static>>, max: usize) -> Vec<Span<'static>> {
    let total: usize = spans.iter().map(|s| markdown::width(&s.content)).sum();
    if total <= max {
        return spans;
    }
    let mut out = Vec::new();
    let mut used = 0;
    for s in spans {
        let w = markdown::width(&s.content);
        if used + w < max {
            used += w;
            out.push(s);
            continue;
        }
        let cut = truncate(&s.content, max - used);
        out.push(Span::styled(cut, s.style));
        break;
    }
    out
}

pub fn truncate(s: &str, max: usize) -> String {
    if markdown::width(s) <= max {
        return s.to_string();
    }
    let mut out = String::new();
    for c in s.chars() {
        if markdown::width(&out) + 2 > max {
            break;
        }
        out.push(c);
    }
    out.push('…');
    out
}

/// The people you work with: handle, who they are, note counts.
pub fn people(people: &crate::people::People, notes: &[crate::people::Note], width: u16) -> Lines {
    if people.people.is_empty() {
        return info(
            "No people yet. Mention someone as @handle and add them: upleveler person add ada --name \"Ada\" --relation mentee",
            width,
        );
    }
    let mut out = Vec::new();
    for p in &people.people {
        let count = notes.iter().filter(|n| n.person == p.handle).count();
        let open = notes
            .iter()
            .filter(|n| n.person == p.handle && n.is_open_follow_up())
            .count();
        let mut meta = format!("{count} {}", if count == 1 { "note" } else { "notes" });
        if open > 0 {
            meta.push_str(&format!(
                " · {open} open follow-up{}",
                if open == 1 { "" } else { "s" }
            ));
        }
        out.push(Line::from(vec![
            Span::styled(format!("  @{:<12} ", p.handle), theme::accent()),
            Span::raw(p.label()),
            Span::styled(format!("  {meta}"), theme::dim()),
        ]));
    }
    out.push(Line::default());
    out
}

/// Notes as `date [kind] text`, newest first.
pub fn notes(notes: &[&crate::people::Note], show_person: bool, width: u16) -> Lines {
    let mut out = Vec::new();
    for n in notes.iter().rev() {
        let who = if show_person {
            format!("@{} ", n.person)
        } else {
            String::new()
        };
        let lead = Span::styled(format!("  {} ", n.date), theme::accent());
        let mut spans = vec![Span::styled(
            format!("{who}[{}] ", n.kind.label()),
            theme::dim(),
        )];
        spans.push(Span::raw(n.text.replace('\n', " / ")));
        if n.kind == crate::people::NoteKind::FollowUp && n.done {
            spans.push(Span::styled(" (done)", theme::dim()));
        }
        out.extend(wrap(
            &spans,
            width as usize,
            &lead,
            &Span::raw("             "),
        ));
    }
    out.push(Line::default());
    out
}

/// Someone's profile, your notes about them and the entries that mention them.
pub fn person(
    p: &crate::people::Person,
    notes_about: &[&crate::people::Note],
    mentioned: &[&Entry],
    width: u16,
) -> Lines {
    let mut out = vec![Line::from(vec![
        Span::styled(format!("  @{} ", p.handle), theme::accent_bold()),
        Span::styled(p.label(), theme::bold()),
    ])];
    for (label, value) in [("team", p.team.as_deref()), ("about", p.about.as_deref())] {
        if let Some(value) = value {
            out.push(Line::from(vec![
                Span::styled(format!("  {label:<6} "), theme::dim()),
                Span::raw(value.to_string()),
            ]));
        }
    }
    out.push(Line::default());
    out.push(Line::styled(
        format!("  Notes ({})", notes_about.len()),
        theme::bold(),
    ));
    out.extend(notes(notes_about, false, width));
    out.push(Line::styled(
        format!("  Entries that mention @{} ({})", p.handle, mentioned.len()),
        theme::bold(),
    ));
    let recent: Vec<&Entry> = mentioned.iter().rev().take(10).rev().copied().collect();
    out.extend(entries(&recent, width));
    out
}

/// Goals with their progress: rating, entries, check-ins, due date.
pub fn goals(
    goals: &[&crate::goals::Goal],
    all_entries: &[Entry],
    gap: Option<&GapSummary>,
    width: u16,
) -> Lines {
    if goals.is_empty() {
        return info("No goals yet. Add one: /goal Speak at a meetup", width);
    }
    let mut out = Vec::new();
    for g in goals {
        let p = crate::goals::progress(g, all_entries, gap);
        let mut facts = Vec::new();
        if let Some(rating) = &p.rating {
            facts.push(format!("{} {rating}", icon(rating)));
        }
        if g.expectation.is_some() || p.tagged > 0 {
            let n = p.evidence + p.tagged;
            facts.push(format!("{n} {}", if n == 1 { "entry" } else { "entries" }));
        }
        if p.checkins > 0 {
            facts.push(format!(
                "{} check-in{}",
                p.checkins,
                if p.checkins == 1 { "" } else { "s" }
            ));
        }
        let lead = Span::styled(format!("  #{:<3}", g.id), theme::accent());
        let mut spans = vec![Span::raw(g.text.clone())];
        if let Some(exp) = &g.expectation {
            spans.push(Span::styled(format!(" [{exp}]"), theme::dim()));
        }
        if let Some(due) = g.due {
            spans.push(Span::styled(format!(" due {due}"), theme::dim()));
        }
        if g.status != crate::goals::GoalStatus::Active {
            spans.push(Span::styled(
                format!(" ({})", g.status.as_str()),
                theme::dim(),
            ));
        }
        if !facts.is_empty() {
            spans.push(Span::styled(
                format!("  · {}", facts.join(" · ")),
                theme::dim(),
            ));
        }
        out.extend(wrap(&spans, width as usize, &lead, &Span::raw("      ")));
    }
    out.push(Line::default());
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyze::GapRow;

    fn summary() -> GapSummary {
        let row = |rating: &str, count| GapRow {
            id: "x".into(),
            area: "Ownership".into(),
            text: "Leads incidents and follows up actions until they are closed".into(),
            rating: rating.into(),
            count,
            last: None,
        };
        GapSummary {
            date: chrono::NaiveDate::from_ymd_opt(2026, 10, 5).unwrap(),
            current: Some("SD2".into()),
            target: "SD3".into(),
            rows: vec![
                row("strong", 4),
                row("partial", 1),
                row("none", 0),
                row("none", 0),
            ],
            overview: "Close.".into(),
            priorities: vec!["Mentor someone".into()],
        }
    }

    #[test]
    fn bar_proportions() {
        let bar: Vec<String> = readiness_bar(&summary(), 8)
            .iter()
            .map(|s| s.content.to_string())
            .collect();
        assert_eq!(bar, ["██", "██", "░░░░"]);
    }

    #[test]
    fn cells_fit_width() {
        let status = Status {
            configured: true,
            entries: 20,
            today: 1,
            last: None,
            streak: 4,
            ladder_levels: Some(5),
            current: Some("SD2".into()),
            target: Some("SD3".into()),
            model: "gemma3:12b".into(),
            base_url: "http://localhost:11434".into(),
            local: true,
            latest_gap: Some(summary()),
        };
        for width in [40u16, 80, 120] {
            let lines = welcome(&status, width);
            let widths: Vec<usize> = lines.iter().map(|l| l.width()).filter(|w| *w > 0).collect();
            assert!(
                widths.windows(2).all(|p| p[0] == p[1]),
                "{width}: {widths:?}"
            );
            assert!(widths[0] <= width as usize);
            for l in gap(&summary(), std::path::Path::new("/tmp/r.md"), width) {
                assert!(
                    l.width() <= width as usize,
                    "{width}: {:?}",
                    markdown::plain(&l)
                );
            }
        }
        let text: Vec<String> = welcome(&status, 80).iter().map(markdown::plain).collect();
        assert!(text
            .iter()
            .any(|l| l.contains("SD2 → SD3") && l.contains("1/4 strong")));
        assert!(text.iter().any(|l| l.contains("🔥 4-day streak")));
    }
}
