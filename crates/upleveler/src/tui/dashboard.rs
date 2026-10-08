//! Full-screen dashboard (alternate screen): overview with heatmap, logs, ladder, reports.

use super::app::Tab;
use super::history::{fit, truncate};
use super::markdown;
use super::theme;
use crate::analyze::GapSummary;
use crate::ladder::Ladder;
use crate::session::{activity, streak, today, ReportFile, Session};
use crate::store::Entry;
use anyhow::Result;
use chrono::{Datelike, Duration, NaiveDate};
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Gauge, Paragraph, Tabs, Wrap};
use ratatui::Frame;
use std::collections::{BTreeMap, HashMap};

const TABS: [Tab; 4] = [Tab::Overview, Tab::Logs, Tab::Ladder, Tab::Reports];
const WEEKS: usize = 26;

pub struct Dashboard {
    pub tab: Tab,
    entries: Vec<Entry>,
    days: BTreeMap<NaiveDate, usize>,
    ladder: Option<Ladder>,
    gap: Option<GapSummary>,
    reports: Vec<ReportFile>,
    target: Option<String>,
    current: Option<String>,
    log_sel: usize,
    filter: String,
    filtering: bool,
    ladder_sel: usize,
    report_sel: usize,
    report_scroll: u16,
    report_cache: Option<(usize, u16, Vec<Line<'static>>)>,
    today: NaiveDate,
    /// Active goals: how many, and the nearest due date.
    goals: (usize, Option<NaiveDate>),
}

impl Dashboard {
    pub fn load(session: &Session, tab: Tab) -> Self {
        let mut entries = session.entries().unwrap_or_default();
        entries.reverse();
        let days = activity(&entries);
        let ladder = session.ladder().ok().flatten();
        let current = session.cfg.current_level.clone();
        let ladder_sel = ladder
            .as_ref()
            .and_then(|l| {
                session
                    .cfg
                    .target_level
                    .as_deref()
                    .and_then(|t| l.levels.iter().position(|x| x.id == t))
            })
            .unwrap_or(0);
        Self {
            tab,
            days,
            entries,
            ladder,
            gap: session.latest_gap(),
            reports: session.reports(),
            target: session.cfg.target_level.clone(),
            current,
            log_sel: 0,
            filter: String::new(),
            filtering: false,
            ladder_sel,
            report_sel: 0,
            report_scroll: 0,
            report_cache: None,
            today: today(),
            goals: session.goals().map_or((0, None), |g| {
                (g.active().count(), g.active().filter_map(|x| x.due).min())
            }),
        }
    }

    fn filtered(&self) -> Vec<&Entry> {
        let f = self.filter.to_lowercase();
        self.entries
            .iter()
            .filter(|e| {
                f.is_empty()
                    || e.text.to_lowercase().contains(&f)
                    || e.tags.iter().any(|t| t.contains(&f))
                    || e.date.to_string().contains(&f)
            })
            .collect()
    }

    /// Handles a key; returns false when the dashboard should close.
    pub fn on_key(&mut self, code: KeyCode, mods: KeyModifiers) -> bool {
        if self.filtering {
            match code {
                KeyCode::Esc => {
                    self.filtering = false;
                    self.filter.clear();
                }
                KeyCode::Enter => self.filtering = false,
                KeyCode::Backspace => {
                    self.filter.pop();
                }
                KeyCode::Char(c) => self.filter.push(c),
                _ => {}
            }
            self.log_sel = 0;
            return true;
        }
        let idx = TABS.iter().position(|t| *t == self.tab).unwrap_or(0);
        match code {
            KeyCode::Esc | KeyCode::Char('q') => return false,
            KeyCode::Char('c') if mods.contains(KeyModifiers::CONTROL) => return false,
            KeyCode::Tab | KeyCode::Right => self.tab = TABS[(idx + 1) % TABS.len()],
            KeyCode::BackTab | KeyCode::Left => {
                self.tab = TABS[(idx + TABS.len() - 1) % TABS.len()]
            }
            KeyCode::Char(c @ '1'..='4') => self.tab = TABS[c as usize - '1' as usize],
            KeyCode::Char('/') if self.tab == Tab::Logs => self.filtering = true,
            KeyCode::Up | KeyCode::Char('k') => self.step(-1),
            KeyCode::Down | KeyCode::Char('j') => self.step(1),
            KeyCode::PageDown | KeyCode::Char(' ') => {
                self.report_scroll = self.report_scroll.saturating_add(10)
            }
            KeyCode::PageUp => self.report_scroll = self.report_scroll.saturating_sub(10),
            _ => {}
        }
        true
    }

    fn step(&mut self, dir: i32) {
        let move_sel = |sel: &mut usize, len: usize| {
            if len == 0 {
                return;
            }
            *sel = if dir < 0 {
                sel.saturating_sub(1)
            } else {
                (*sel + 1).min(len - 1)
            };
        };
        match self.tab {
            Tab::Logs => {
                let len = self.filtered().len();
                move_sel(&mut self.log_sel, len);
            }
            Tab::Ladder => {
                let len = self.ladder.as_ref().map_or(0, |l| l.levels.len());
                move_sel(&mut self.ladder_sel, len);
            }
            Tab::Reports => {
                let before = self.report_sel;
                move_sel(&mut self.report_sel, self.reports.len());
                if before != self.report_sel {
                    self.report_scroll = 0;
                }
            }
            Tab::Overview => {}
        }
    }

    pub fn draw(&mut self, frame: &mut Frame) {
        let area = frame.area();
        let [header, body, footer] = Layout::vertical([
            Constraint::Length(2),
            Constraint::Min(5),
            Constraint::Length(1),
        ])
        .areas(area);
        let titles: Vec<Line> = ["1 Overview", "2 Logs", "3 Ladder", "4 Reports"]
            .iter()
            .map(|t| Line::from(*t))
            .collect();
        let idx = TABS.iter().position(|t| *t == self.tab).unwrap_or(0);
        let [tabs_area, brand] =
            Layout::horizontal([Constraint::Min(10), Constraint::Length(14)]).areas(header);
        frame.render_widget(
            Tabs::new(titles)
                .select(idx)
                .style(theme::dim())
                .highlight_style(theme::accent_bold())
                .divider(Span::styled("│", theme::dim()))
                .block(
                    Block::default()
                        .borders(Borders::BOTTOM)
                        .border_style(theme::dim()),
                ),
            tabs_area,
        );
        frame.render_widget(
            Paragraph::new(Span::styled("▲ upleveler", theme::accent_bold()))
                .right_aligned()
                .block(
                    Block::default()
                        .borders(Borders::BOTTOM)
                        .border_style(theme::dim()),
                ),
            brand,
        );
        match self.tab {
            Tab::Overview => self.overview(frame, body),
            Tab::Logs => self.logs(frame, body),
            Tab::Ladder => self.ladder_tab(frame, body),
            Tab::Reports => self.reports_tab(frame, body),
        }
        let hint = match (self.tab, self.filtering) {
            (_, true) => "type to filter · enter keep · esc clear".to_string(),
            (Tab::Logs, _) => "↑↓ move · / filter · tab next tab · esc back".to_string(),
            (Tab::Reports, _) => {
                "↑↓ choose · space/pgdn scroll · tab next tab · esc back".to_string()
            }
            _ => "tab / 1-4 switch · ↑↓ move · esc back".to_string(),
        };
        frame.render_widget(Paragraph::new(Span::styled(hint, theme::dim())), footer);
    }

    fn panel(title: &str) -> Block<'static> {
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(theme::dim())
            .title(Span::styled(format!(" {title} "), theme::accent_bold()))
    }

    fn overview(&self, frame: &mut Frame, area: Rect) {
        let [top, middle, heat] = Layout::vertical([
            Constraint::Length(4),
            Constraint::Min(6),
            Constraint::Length(12),
        ])
        .areas(area);

        let target = self.target.clone().unwrap_or_else(|| "?".into());
        let title = format!("Readiness for {target}");
        match &self.gap {
            Some(g) if !g.rows.is_empty() => {
                let ratio = g.count("strong") as f64 / g.rows.len() as f64;
                let label = format!(
                    "{} strong · {} partial · {} missing   (gap {})",
                    g.count("strong"),
                    g.count("partial"),
                    g.count("none"),
                    g.date
                );
                frame.render_widget(
                    Gauge::default()
                        .block(Self::panel(&title))
                        .gauge_style(theme::good())
                        .ratio(ratio.clamp(0.0, 1.0))
                        .label(Span::styled(label, theme::bold())),
                    top,
                );
            }
            _ => frame.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::styled("No gap analysis yet · run ", theme::dim()),
                    Span::styled("/gap", theme::accent()),
                    Span::styled(" to see where you stand", theme::dim()),
                ]))
                .block(Self::panel(&title)),
                top,
            ),
        }

        let [areas, stats] =
            Layout::horizontal([Constraint::Percentage(60), Constraint::Percentage(40)])
                .areas(middle);
        let mut lines = Vec::new();
        if let Some(g) = &self.gap {
            let mut by_area: Vec<(String, Vec<&str>)> = Vec::new();
            for r in &g.rows {
                match by_area.iter_mut().find(|(a, _)| *a == r.area) {
                    Some((_, v)) => v.push(&r.rating),
                    None => by_area.push((r.area.clone(), vec![&r.rating])),
                }
            }
            let w = by_area
                .iter()
                .map(|(a, _)| markdown::width(a))
                .max()
                .unwrap_or(4)
                .min(20);
            for (a, ratings) in by_area {
                let name = truncate(&a, w);
                let pad = " ".repeat(w.saturating_sub(markdown::width(&name)));
                let mut spans = vec![Span::raw(format!(" {name}{pad}  "))];
                for r in &ratings {
                    spans.push(Span::styled("■ ", theme::rating(r)));
                }
                let strong = ratings.iter().filter(|r| **r == "strong").count();
                spans.push(Span::styled(
                    format!(" {strong}/{}", ratings.len()),
                    theme::dim(),
                ));
                lines.push(Line::from(spans));
            }
            if !g.priorities.is_empty() {
                lines.push(Line::default());
                lines.push(Line::styled(" Next", theme::bold()));
                for p in g.priorities.iter().take(3) {
                    lines.push(Line::from(vec![
                        Span::styled(" → ", theme::accent()),
                        Span::raw(p.clone()),
                    ]));
                }
            }
        } else {
            lines.push(Line::styled(
                " Areas appear here after your first /gap.",
                theme::dim(),
            ));
        }
        frame.render_widget(
            Paragraph::new(lines)
                .wrap(Wrap { trim: false })
                .block(Self::panel("By area")),
            areas,
        );

        let week_start =
            self.today - Duration::days(self.today.weekday().num_days_from_monday() as i64);
        let month_start = self.today.with_day(1).unwrap_or(self.today);
        let count_since =
            |from: NaiveDate| -> usize { self.days.range(from..).map(|(_, n)| n).sum() };
        let mut tags: HashMap<&str, usize> = HashMap::new();
        for e in &self.entries {
            for t in &e.tags {
                *tags.entry(t).or_default() += 1;
            }
        }
        let mut tags: Vec<_> = tags.into_iter().collect();
        tags.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
        let stat = |k: &str, v: String| {
            Line::from(vec![
                Span::styled(format!(" {k:<12}"), theme::dim()),
                Span::styled(v, theme::bold()),
            ])
        };
        let mut lines = vec![
            stat("entries", self.entries.len().to_string()),
            stat("this week", count_since(week_start).to_string()),
            stat("this month", count_since(month_start).to_string()),
            stat(
                "streak",
                format!("{} days 🔥", streak(&self.days, self.today)),
            ),
            stat(
                "goals",
                match self.goals {
                    (0, _) => "none · /goal to add one".to_string(),
                    (n, Some(due)) => format!("{n} active · next due {due}"),
                    (n, None) => format!("{n} active"),
                },
            ),
            stat(
                "last entry",
                self.entries
                    .first()
                    .map_or("—".into(), |e| e.date.to_string()),
            ),
        ];
        if !tags.is_empty() {
            lines.push(Line::default());
            let top: Vec<String> = tags
                .iter()
                .take(5)
                .map(|(t, n)| format!("{t} {n}"))
                .collect();
            lines.push(Line::from(vec![
                Span::styled(" top tags    ", theme::dim()),
                Span::raw(top.join(" · ")),
            ]));
        }
        frame.render_widget(
            Paragraph::new(lines)
                .wrap(Wrap { trim: false })
                .block(Self::panel("Activity")),
            stats,
        );

        frame.render_widget(
            Paragraph::new(heatmap(
                &self.days,
                self.today,
                heat.width.saturating_sub(8) as usize,
            ))
            .block(Self::panel("Last 26 weeks")),
            heat,
        );
    }

    fn logs(&self, frame: &mut Frame, area: Rect) {
        let [list_area, detail] =
            Layout::horizontal([Constraint::Percentage(60), Constraint::Percentage(40)])
                .areas(area);
        let items = self.filtered();
        let height = list_area.height.saturating_sub(2) as usize;
        let sel = self.log_sel.min(items.len().saturating_sub(1));
        let start = sel.saturating_sub(height.saturating_sub(1));
        let w = list_area.width.saturating_sub(4) as usize;
        let lines: Vec<Line> = items
            .iter()
            .enumerate()
            .skip(start)
            .take(height)
            .map(|(i, e)| {
                let style = if i == sel {
                    theme::selected()
                } else {
                    Style::default()
                };
                Line::from(fit(
                    vec![
                        Span::styled(
                            format!("{} ", e.date),
                            if i == sel { style } else { theme::accent() },
                        ),
                        Span::styled(e.text.replace('\n', " / "), style),
                    ],
                    w,
                ))
            })
            .collect();
        let title = if self.filter.is_empty() && !self.filtering {
            format!("Logs · {}", items.len())
        } else {
            format!("Logs · {} matching “{}”", items.len(), self.filter)
        };
        frame.render_widget(Paragraph::new(lines).block(Self::panel(&title)), list_area);

        let mut lines = Vec::new();
        if let Some(e) = items.get(sel) {
            lines.push(Line::styled(
                e.date.format("%A, %d %B %Y").to_string(),
                theme::accent_bold(),
            ));
            lines.push(Line::default());
            lines.push(Line::raw(e.text.clone()));
            lines.push(Line::default());
            if !e.tags.is_empty() {
                lines.push(Line::from(vec![
                    Span::styled("tags  ", theme::dim()),
                    Span::raw(e.tags.join(", ")),
                ]));
            }
            for l in &e.links {
                lines.push(Line::from(vec![
                    Span::styled("link  ", theme::dim()),
                    Span::styled(l.clone(), theme::accent()),
                ]));
            }
            lines.push(Line::from(vec![
                Span::styled("from  ", theme::dim()),
                Span::raw(e.source.clone()),
            ]));
            if !e.expectations.is_empty() {
                lines.push(Line::default());
                lines.push(Line::styled("Evidence for", theme::bold()));
                for id in &e.expectations {
                    let text = self
                        .ladder
                        .as_ref()
                        .and_then(|l| l.expectation(id))
                        .map_or(id.clone(), |x| x.text.clone());
                    lines.push(Line::from(vec![
                        Span::styled("• ", theme::good()),
                        Span::raw(text),
                    ]));
                }
            }
        } else {
            lines.push(Line::styled("No entries.", theme::dim()));
        }
        frame.render_widget(
            Paragraph::new(lines)
                .wrap(Wrap { trim: false })
                .block(Self::panel("Entry")),
            detail,
        );
    }

    fn ladder_tab(&self, frame: &mut Frame, area: Rect) {
        let Some(ladder) = &self.ladder else {
            frame.render_widget(
                Paragraph::new(Span::styled(
                    " No ladder yet · /ladder import @file",
                    theme::dim(),
                ))
                .block(Self::panel("Ladder")),
                area,
            );
            return;
        };
        let [list_area, detail] =
            Layout::horizontal([Constraint::Length(38), Constraint::Min(20)]).areas(area);
        let lines: Vec<Line> = ladder
            .levels
            .iter()
            .enumerate()
            .map(|(i, l)| {
                let mark = if self.current.as_deref() == Some(&l.id) {
                    " ● you"
                } else if self.target.as_deref() == Some(&l.id) {
                    " ◎ target"
                } else {
                    ""
                };
                let style = if i == self.ladder_sel {
                    theme::selected()
                } else {
                    Style::default()
                };
                Line::from(vec![
                    Span::styled(format!(" {} ", l.id), style),
                    Span::styled(truncate(&l.title, 18), style),
                    Span::styled(mark, theme::accent()),
                ])
            })
            .collect();
        frame.render_widget(
            Paragraph::new(lines).block(Self::panel("Levels")),
            list_area,
        );

        let Some(level) = ladder.levels.get(self.ladder_sel) else {
            return;
        };
        let mut counts: HashMap<&str, usize> = HashMap::new();
        for e in &self.entries {
            for id in &e.expectations {
                *counts.entry(id.as_str()).or_default() += 1;
            }
        }
        let ratings: HashMap<&str, &str> = self
            .gap
            .iter()
            .flat_map(|g| g.rows.iter().map(|r| (r.id.as_str(), r.rating.as_str())))
            .collect();
        let mut lines = Vec::new();
        if let Some(s) = &level.summary {
            lines.push(Line::styled(s.clone(), theme::dim()));
            lines.push(Line::default());
        }
        let mut area_name = "";
        for e in &level.expectations {
            if e.area != area_name {
                area_name = &e.area;
                lines.push(Line::styled(area_name.to_string(), theme::bold()));
            }
            let n = counts.get(e.id.as_str()).copied().unwrap_or(0);
            let rating = ratings.get(e.id.as_str()).copied().unwrap_or("");
            let dot = match rating {
                "strong" => "●",
                "partial" => "◐",
                "none" => "○",
                _ => "·",
            };
            lines.push(Line::from(vec![
                Span::styled(format!(" {dot} "), theme::rating(rating)),
                Span::raw(e.text.clone()),
                Span::styled(format!("  {n} entries"), theme::dim()),
            ]));
        }
        frame.render_widget(
            Paragraph::new(lines)
                .wrap(Wrap { trim: false })
                .block(Self::panel(&format!("{} — {}", level.id, level.title))),
            detail,
        );
    }

    fn reports_tab(&mut self, frame: &mut Frame, area: Rect) {
        if self.reports.is_empty() {
            frame.render_widget(
                Paragraph::new(Span::styled(
                    " No reports yet · try /gap, /brag or /summary",
                    theme::dim(),
                ))
                .block(Self::panel("Reports")),
                area,
            );
            return;
        }
        let [list_area, preview] =
            Layout::horizontal([Constraint::Length(36), Constraint::Min(20)]).areas(area);
        let lines: Vec<Line> = self
            .reports
            .iter()
            .enumerate()
            .map(|(i, r)| {
                let style = if i == self.report_sel {
                    theme::selected()
                } else {
                    Style::default()
                };
                Line::styled(format!(" {}", truncate(&r.name, 32)), style)
            })
            .collect();
        frame.render_widget(
            Paragraph::new(lines).block(Self::panel("Reports")),
            list_area,
        );

        let width = preview.width.saturating_sub(4);
        let stale = self
            .report_cache
            .as_ref()
            .is_none_or(|(sel, w, _)| *sel != self.report_sel || *w != width);
        if stale {
            let md =
                std::fs::read_to_string(&self.reports[self.report_sel].path).unwrap_or_default();
            self.report_cache = Some((self.report_sel, width, markdown::render(&md, width)));
        }
        let lines = self
            .report_cache
            .as_ref()
            .map(|c| c.2.clone())
            .unwrap_or_default();
        let max_scroll = (lines.len() as u16).saturating_sub(preview.height.saturating_sub(2));
        self.report_scroll = self.report_scroll.min(max_scroll);
        frame.render_widget(
            Paragraph::new(lines)
                .scroll((self.report_scroll, 0))
                .block(Self::panel(&self.reports[self.report_sel].name)),
            preview,
        );
    }
}

/// GitHub-style contribution grid: 7 weekday rows × up to 26 weeks, newest on the right.
pub fn heatmap(
    days: &BTreeMap<NaiveDate, usize>,
    today: NaiveDate,
    width: usize,
) -> Vec<Line<'static>> {
    let weeks = (width / 2).clamp(4, WEEKS);
    let this_monday = today - Duration::days(today.weekday().num_days_from_monday() as i64);
    let first_monday = this_monday - Duration::weeks(weeks as i64 - 1);
    let mut lines = Vec::new();

    let mut months = String::from("    ");
    let mut last_month = 0;
    let mut col = 0;
    while col < weeks {
        let monday = first_monday + Duration::weeks(col as i64);
        if monday.month() != last_month {
            last_month = monday.month();
            let name = monday.format("%b").to_string();
            months.push_str(&name);
            months.push(' ');
            col += 2;
        } else {
            months.push_str("  ");
            col += 1;
        }
    }
    lines.push(Line::styled(months, theme::dim()));

    for (row, label) in ["Mon", "", "Wed", "", "Fri", "", "Sun"].iter().enumerate() {
        let mut spans = vec![Span::styled(format!("{label:<4}"), theme::dim())];
        for w in 0..weeks {
            let day = first_monday + Duration::weeks(w as i64) + Duration::days(row as i64);
            if day > today {
                spans.push(Span::raw("  "));
            } else {
                let n = days.get(&day).copied().unwrap_or(0);
                spans.push(Span::styled("■ ", theme::heat(n)));
            }
        }
        lines.push(Line::from(spans));
    }
    let total: usize = days.range(first_monday..).map(|(_, n)| n).sum();
    let mut legend = vec![Span::styled(
        format!("    {total} entries · less "),
        theme::dim(),
    )];
    for n in 0..=4 {
        legend.push(Span::styled("■ ", theme::heat(n)));
    }
    legend.push(Span::styled("more", theme::dim()));
    lines.push(Line::default());
    lines.push(Line::from(legend));
    lines
}

/// Runs the dashboard until the user leaves it. The caller has switched to the
/// alternate screen and passes a full-screen terminal.
pub fn run<B: ratatui::backend::Backend>(
    terminal: &mut ratatui::Terminal<B>,
    dash: &mut Dashboard,
) -> Result<()> {
    loop {
        terminal.draw(|f| dash.draw(f))?;
        if let Event::Key(key) = event::read()? {
            if key.kind != KeyEventKind::Press {
                continue;
            }
            if !dash.on_key(key.code, key.modifiers) {
                return Ok(());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Paths;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn screen(term: &Terminal<TestBackend>) -> String {
        let buf = term.backend().buffer();
        (0..buf.area.height)
            .map(|y| {
                (0..buf.area.width)
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn heatmap_shape() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 7).unwrap(); // Wednesday
        let mut days = BTreeMap::new();
        days.insert(today, 3);
        days.insert(today - Duration::days(1), 1);
        let lines = heatmap(&days, today, 52);
        assert_eq!(lines.len(), 10);
        assert!(markdown::plain(&lines[1]).starts_with("Mon "));
        // Thursday onwards of this week is in the future: blank.
        let thu = markdown::plain(&lines[4]);
        assert!(thu.ends_with("  "), "{thu:?}");
        assert!(markdown::plain(&lines[9]).contains("4 entries"));
    }

    #[test]
    fn tabs_render() {
        let dir = tempfile::tempdir().unwrap();
        let mut session = Session::at(Paths::at(dir.path().to_path_buf())).unwrap();
        session.cfg.target_level = Some("L3".into());
        session.save_config().unwrap();
        crate::ladder::Ladder::from_yaml(super::super::app::EXAMPLE_LADDER)
            .unwrap()
            .save(&session.paths.ladder)
            .unwrap();
        session
            .add_log(
                "Led the payment outage call",
                today(),
                vec!["incident".into()],
            )
            .unwrap();
        std::fs::create_dir_all(&session.paths.reports).unwrap();
        std::fs::write(
            session.paths.reports.join("gap-2026-10-05.md"),
            "# Gap\n\n- **one**",
        )
        .unwrap();

        let mut dash = Dashboard::load(&session, Tab::Overview);
        let mut term = Terminal::new(TestBackend::new(100, 34)).unwrap();
        term.draw(|f| dash.draw(f)).unwrap();
        let s = screen(&term);
        assert!(
            s.contains("Readiness for L3") && s.contains("No gap analysis yet"),
            "{s}"
        );
        assert!(s.contains("Last 26 weeks") && s.contains("streak"), "{s}");

        dash.on_key(KeyCode::Char('2'), KeyModifiers::NONE);
        term.draw(|f| dash.draw(f)).unwrap();
        let s = screen(&term);
        assert!(
            s.contains("Logs · 1") && s.contains("Led the payment outage call"),
            "{s}"
        );

        dash.on_key(KeyCode::Char('/'), KeyModifiers::NONE);
        for c in "zzz".chars() {
            dash.on_key(KeyCode::Char(c), KeyModifiers::NONE);
        }
        term.draw(|f| dash.draw(f)).unwrap();
        assert!(screen(&term).contains("0 matching"));
        dash.on_key(KeyCode::Esc, KeyModifiers::NONE);

        dash.on_key(KeyCode::Char('3'), KeyModifiers::NONE);
        term.draw(|f| dash.draw(f)).unwrap();
        let s = screen(&term);
        assert!(
            s.contains("L3 — Senior Engineer") && s.contains("◎ target"),
            "{s}"
        );

        dash.on_key(KeyCode::Char('4'), KeyModifiers::NONE);
        term.draw(|f| dash.draw(f)).unwrap();
        let s = screen(&term);
        assert!(s.contains("gap-2026-10-05") && s.contains("• one"), "{s}");
        assert!(!dash.on_key(KeyCode::Esc, KeyModifiers::NONE));
    }
}
