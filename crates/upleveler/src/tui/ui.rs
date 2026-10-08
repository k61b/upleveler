//! Drawing the live area at the bottom of the terminal.

use super::app::{App, Panel, Popup};
use super::history::{fit, truncate};
use super::markdown;
use super::theme;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph, Widget, Wrap};
use ratatui::Frame;

const SPINNER: &[&str] = &["·", "✢", "✳", "✶", "✻", "✽", "✻", "✶", "✳", "✢"];
const MAX_COMPOSER_LINES: usize = 6;
const STREAM_PREVIEW_LINES: usize = 10;

fn panel_lines(app: &App, width: u16) -> Vec<Line<'static>> {
    let w = width as usize;
    let mut lines: Vec<Line<'static>> = Vec::new();
    if let Some(panel) = &app.panel {
        match panel {
            Panel::Help => {
                lines.push(Line::styled("Commands", theme::accent_bold()));
                for c in super::commands::COMMANDS {
                    lines.push(Line::from(fit(
                        vec![
                            Span::styled(format!("  /{:<10}", c.name), theme::accent()),
                            Span::styled(format!("{:<28}", c.args), theme::dim()),
                            Span::raw(c.help),
                        ],
                        w,
                    )));
                }
                lines.push(Line::default());
                lines.push(Line::styled("Keys", theme::accent_bold()));
                for row in [
                    [
                        ("enter", "send"),
                        ("shift+enter", "new line"),
                        ("↑ ↓", "history, lists"),
                    ],
                    [
                        ("tab", "complete /, @person, @file"),
                        ("esc", "close, stop, clear"),
                        ("ctrl+c ×2", "exit"),
                    ],
                ] {
                    let mut spans = vec![Span::raw("  ")];
                    for (k, what) in row {
                        spans.push(Span::styled(format!("{k} "), theme::accent()));
                        spans.push(Span::raw(format!("{what}   ")));
                    }
                    lines.push(Line::from(fit(spans, w)));
                }
                lines.push(Line::styled("  press any key to close", theme::dim()));
            }
            Panel::Choice(text) => {
                let person = app.mentioned_person(text);
                let title = if person.is_some() {
                    "Should I log this, note it about them, or answer it?"
                } else {
                    "Should I log this, or answer it?"
                };
                lines.push(Line::styled(title, theme::bold()));
                lines.push(Line::styled(
                    format!("  “{}”", truncate(text, w.saturating_sub(4))),
                    theme::dim(),
                ));
                let mut keys = vec![
                    Span::styled("  [l]", theme::accent_bold()),
                    Span::raw(" log it   "),
                ];
                if let Some(person) = person {
                    keys.push(Span::styled("[n]", theme::accent_bold()));
                    keys.push(Span::raw(format!(" note about @{person}   ")));
                }
                keys.extend([
                    Span::styled("[a]", theme::accent_bold()),
                    Span::raw(" ask   "),
                    Span::styled("[esc]", theme::dim()),
                    Span::styled(" cancel", theme::dim()),
                ]);
                lines.push(Line::from(keys));
            }
            Panel::Select {
                title,
                hint,
                items,
                selected,
                ..
            } => {
                lines.push(Line::styled(title.clone(), theme::bold()));
                for l in markdown::wrap(
                    &[Span::styled(hint.clone(), theme::dim())],
                    w,
                    &Span::raw(""),
                    &Span::raw(""),
                ) {
                    lines.push(l);
                }
                let visible = 8usize;
                let start = selected.saturating_sub(visible - 1);
                for (i, item) in items.iter().enumerate().skip(start).take(visible) {
                    let mark = if i == *selected { "❯ " } else { "  " };
                    let style = if i == *selected {
                        theme::accent_bold()
                    } else {
                        Style::default()
                    };
                    lines.push(Line::from(fit(
                        vec![
                            Span::styled(format!("{mark}{}", item.label), style),
                            Span::styled(
                                if item.detail.is_empty() {
                                    String::new()
                                } else {
                                    format!("  {}", item.detail)
                                },
                                theme::dim(),
                            ),
                        ],
                        w,
                    )));
                }
            }
            Panel::Input { title, hint, .. } => {
                lines.push(Line::styled(title.clone(), theme::bold()));
                lines.extend(markdown::wrap(
                    &[Span::styled(hint.clone(), theme::dim())],
                    w,
                    &Span::raw(""),
                    &Span::raw(""),
                ));
                // The input box itself is drawn separately below these lines.
            }
            Panel::LadderSheets {
                sheets, selected, ..
            } => {
                use crate::ladder::SheetRole;
                lines.push(Line::styled("What is each sheet for?", theme::bold()));
                lines.push(Line::styled(
                    "  ↑↓ choose a sheet · ←→ change its role · enter read the ladder · esc cancel",
                    theme::dim(),
                ));
                let name_width = sheets
                    .iter()
                    .map(|s| s.name.chars().count())
                    .max()
                    .unwrap_or(0)
                    .min(24);
                for (i, s) in sheets.iter().enumerate() {
                    let mark = if i == *selected { "❯ " } else { "  " };
                    let style = if i == *selected {
                        theme::accent_bold()
                    } else {
                        Style::default()
                    };
                    let found = match s.role {
                        SheetRole::Expectations if !s.levels.is_empty() => {
                            format!("{} · {} expectations", s.levels.join(", "), s.items)
                        }
                        SheetRole::Expectations => "the model reads it".to_string(),
                        SheetRole::Skip => format!("{} rows", s.rows),
                        _ => s.levels.join(", "),
                    };
                    lines.push(Line::from(fit(
                        vec![
                            Span::styled(
                                format!("{mark}{:<name_width$}  ", truncate(&s.name, name_width)),
                                style,
                            ),
                            Span::styled(format!("{:<16}", s.role.label()), theme::accent()),
                            Span::styled(found, theme::dim()),
                        ],
                        w,
                    )));
                }
            }
            Panel::Sheets {
                sheets, selected, ..
            } => {
                lines.push(Line::styled("Where should each sheet go?", theme::bold()));
                lines.push(Line::styled(
                    "  ↑↓ choose a sheet · ←→ change where it goes · enter read the file · esc cancel",
                    theme::dim(),
                ));
                let name_width = sheets
                    .iter()
                    .map(|s| s.name.chars().count())
                    .max()
                    .unwrap_or(0)
                    .min(24);
                for (i, s) in sheets.iter().enumerate() {
                    let mark = if i == *selected { "❯ " } else { "  " };
                    let style = if i == *selected {
                        theme::accent_bold()
                    } else {
                        Style::default()
                    };
                    lines.push(Line::from(fit(
                        vec![
                            Span::styled(
                                format!("{mark}{:<name_width$}  ", truncate(&s.name, name_width)),
                                style,
                            ),
                            Span::styled(format!("{:<22}", s.kind.label()), theme::accent()),
                            Span::styled(format!("{} rows", s.rows), theme::dim()),
                        ],
                        w,
                    )));
                }
            }
            Panel::Import { preview, scroll } => {
                let plan = &preview.plan;
                lines.push(Line::from(vec![
                    Span::styled("Import ", theme::bold()),
                    Span::styled(
                        preview.file.file_name().map_or_else(
                            || preview.file.display().to_string(),
                            |n| n.to_string_lossy().into_owned(),
                        ),
                        theme::accent(),
                    ),
                ]));
                let span = match (plan.new.first(), plan.new.last()) {
                    (Some(a), Some(b)) => format!("  {} → {}", a.date, b.date),
                    _ => String::new(),
                };
                let has_log = !preview.collected.drafts.is_empty();
                if has_log {
                    lines.push(Line::from(vec![
                        Span::styled(
                            format!("  {} new log entries", plan.new.len()),
                            theme::good(),
                        ),
                        Span::styled(span, theme::dim()),
                        Span::styled(
                            format!(
                                " · {} duplicates · {} without a date",
                                plan.duplicates,
                                plan.undated.len(),
                            ),
                            theme::dim(),
                        ),
                    ]));
                    let shown = if preview.notes.is_empty() && preview.goals.is_empty() {
                        6
                    } else {
                        3
                    };
                    for e in plan.new.iter().skip(*scroll).take(shown) {
                        lines.push(Line::from(fit(
                            vec![
                                Span::styled(format!("    {} ", e.date), theme::accent()),
                                Span::raw(e.text.replace('\n', " / ")),
                            ],
                            w,
                        )));
                    }
                }
                if !preview.notes.is_empty() || preview.note_duplicates > 0 {
                    let follow_ups = preview
                        .notes
                        .iter()
                        .filter(|n| n.kind == crate::people::NoteKind::FollowUp)
                        .count();
                    lines.push(Line::from(vec![
                        Span::styled(
                            format!(
                                "  {} new notes about @{}",
                                preview.notes.len(),
                                preview.person.as_deref().unwrap_or("?")
                            ),
                            theme::good(),
                        ),
                        Span::styled(
                            format!(
                                " · {follow_ups} follow-ups · {} already there",
                                preview.note_duplicates
                            ),
                            theme::dim(),
                        ),
                    ]));
                    for n in preview.notes.iter().take(3) {
                        lines.push(Line::from(fit(
                            vec![
                                Span::styled(format!("    {} ", n.date), theme::accent()),
                                Span::styled(format!("{} ", n.kind.label()), theme::dim()),
                                Span::raw(n.text.replace('\n', " / ")),
                            ],
                            w,
                        )));
                    }
                }
                if !preview.goals.is_empty() || preview.goal_duplicates > 0 {
                    let tied = preview
                        .goals
                        .iter()
                        .filter(|g| g.expectation.is_some())
                        .count();
                    lines.push(Line::from(vec![
                        Span::styled(
                            format!("  {} new goals", preview.goals.len()),
                            theme::good(),
                        ),
                        Span::styled(
                            format!(
                                " · {tied} tied to your ladder · {} already there",
                                preview.goal_duplicates
                            ),
                            theme::dim(),
                        ),
                    ]));
                    for g in preview.goals.iter().take(3) {
                        lines.push(Line::from(fit(
                            vec![
                                Span::styled(
                                    format!(
                                        "    {} ",
                                        g.expectation.as_deref().unwrap_or("free goal")
                                    ),
                                    theme::accent(),
                                ),
                                Span::raw(g.text.clone()),
                            ],
                            w,
                        )));
                    }
                }
                if !preview.collected.warnings.is_empty() {
                    lines.push(Line::styled(
                        format!(
                            "  {} warnings, printed above",
                            preview.collected.warnings.len()
                        ),
                        theme::dim(),
                    ));
                }
                lines.push(Line::from(vec![
                    Span::styled("  [enter]", theme::accent_bold()),
                    Span::raw(" add them   "),
                    Span::styled("[e]", theme::accent_bold()),
                    Span::raw(" edit in $EDITOR   "),
                    Span::styled("[↑↓]", theme::dim()),
                    Span::styled(" scroll   ", theme::dim()),
                    Span::styled("[esc]", theme::dim()),
                    Span::styled(" cancel", theme::dim()),
                ]));
            }
            Panel::Ladder {
                ladder,
                file,
                scroll,
            } => {
                lines.push(Line::from(vec![
                    Span::styled("Ladder from ", theme::bold()),
                    Span::styled(file.display().to_string(), theme::accent()),
                ]));
                let mut body: Vec<Line<'static>> = Vec::new();
                for l in &ladder.levels {
                    body.push(Line::from(vec![
                        Span::styled(format!("  {} ", l.id), theme::accent_bold()),
                        Span::raw(l.title.clone()),
                        Span::styled(
                            format!("  {} expectations", l.expectations.len()),
                            theme::dim(),
                        ),
                    ]));
                    for e in &l.expectations {
                        body.push(Line::from(fit(
                            vec![
                                Span::styled(format!("      {} · ", e.area), theme::dim()),
                                Span::raw(e.text.clone()),
                            ],
                            w,
                        )));
                    }
                }
                let start = (*scroll).min(body.len().saturating_sub(1));
                lines.extend(body.into_iter().skip(start).take(8));
                lines.push(Line::from(vec![
                    Span::styled("  [enter]", theme::accent_bold()),
                    Span::raw(" save   "),
                    Span::styled("[↑↓]", theme::dim()),
                    Span::styled(" scroll   ", theme::dim()),
                    Span::styled("[esc]", theme::dim()),
                    Span::styled(" cancel", theme::dim()),
                ]));
            }
        }
        return lines;
    }
    if let Some(popup) = app.popup() {
        let selected = app.popup_index;
        match popup {
            Popup::Commands(list) => {
                let start = selected.saturating_sub(7);
                for (i, c) in list.iter().enumerate().skip(start).take(8) {
                    let on = i == selected % list.len();
                    let name_style = if on {
                        theme::accent_bold()
                    } else {
                        theme::accent()
                    };
                    lines.push(Line::from(fit(
                        vec![
                            Span::styled(if on { "❯ " } else { "  " }, theme::accent()),
                            Span::styled(format!("/{:<10}", c.name), name_style),
                            Span::styled(
                                format!("{} ", c.help),
                                if on { Style::default() } else { theme::dim() },
                            ),
                            Span::styled(c.args, theme::dim()),
                        ],
                        w,
                    )));
                }
            }
            Popup::People(list) => {
                for (i, (handle, label)) in list.iter().enumerate() {
                    let on = i == selected % list.len();
                    lines.push(Line::from(vec![
                        Span::styled(if on { "❯ " } else { "  " }, theme::accent()),
                        Span::styled(
                            format!("@{handle:<14}"),
                            if on {
                                theme::accent_bold()
                            } else {
                                theme::accent()
                            },
                        ),
                        Span::styled(label.clone(), theme::dim()),
                    ]));
                }
            }
            Popup::Files(list) => {
                for (i, c) in list.iter().enumerate() {
                    let on = i == selected % list.len();
                    lines.push(Line::from(vec![
                        Span::styled(if on { "❯ " } else { "  " }, theme::accent()),
                        Span::styled(
                            c.value.clone(),
                            if on {
                                theme::accent_bold()
                            } else if c.is_dir {
                                theme::accent()
                            } else {
                                Style::default()
                            },
                        ),
                    ]));
                }
            }
        }
        return lines;
    }
    if let Some(job) = &app.job {
        if !job.streamed.is_empty() {
            let rendered = markdown::render(&job.streamed, width.saturating_sub(2));
            let skip = rendered.len().saturating_sub(STREAM_PREVIEW_LINES);
            for l in rendered.into_iter().skip(skip) {
                let mut spans = vec![Span::raw("  ")];
                spans.extend(l.spans);
                lines.push(Line::from(spans));
            }
        }
    }
    lines
}

fn composer_height(app: &App) -> u16 {
    let n = match &app.panel {
        Some(Panel::Input { .. }) => 1,
        _ => app.composer.lines().len().clamp(1, MAX_COMPOSER_LINES),
    };
    n as u16 + 2
}

/// How tall the live area wants to be, capped by the screen.
pub fn desired_height(app: &App, width: u16, screen: u16) -> u16 {
    let panel = panel_lines(app, width.saturating_sub(2)).len() as u16;
    let status = 1;
    let footer = 1;
    (status + panel + composer_height(app) + footer)
        .min(screen.saturating_sub(1))
        .max(5)
}

fn status_line(app: &App, width: u16) -> Line<'static> {
    if let Some(job) = &app.job {
        let elapsed = job.started.elapsed();
        let frame = SPINNER[(elapsed.as_millis() / 120) as usize % SPINNER.len()];
        let mut spans = vec![
            Span::styled(format!("{frame} "), theme::accent_bold()),
            Span::styled(format!("{}…", job.label), theme::accent()),
        ];
        if let Some((label, done, total)) = &job.progress {
            let cells = 10usize;
            let filled = if *total == 0 { 0 } else { done * cells / total };
            spans.push(Span::styled(format!("  {label} "), theme::dim()));
            spans.push(Span::styled("▰".repeat(filled), theme::accent()));
            spans.push(Span::styled("▱".repeat(cells - filled), theme::dim()));
            spans.push(Span::styled(format!(" {done}/{total}"), theme::dim()));
        }
        let hint = if job.cancelling {
            "stopping after this step".to_string()
        } else {
            "esc to interrupt".to_string()
        };
        spans.push(Span::styled(
            format!("  ({}s · {hint})", elapsed.as_secs()),
            theme::dim(),
        ));
        return Line::from(fit(spans, width as usize));
    }
    if let Some((text, at)) = &app.notice {
        if at.elapsed().as_secs() < 3 {
            return Line::styled(text.clone(), theme::warn());
        }
    }
    Line::default()
}

fn footer(app: &App, width: u16) -> (Line<'static>, Line<'static>) {
    let left = if app.panel.is_some() {
        ""
    } else if app.popup().is_some() {
        "↑↓ select · tab complete · enter run · esc close"
    } else if app.job.is_some() {
        "you can keep logging while this runs"
    } else {
        "? shortcuts · / commands · @ files"
    };
    let s = &app.status;
    let levels = match (&s.current, &s.target) {
        (Some(c), Some(t)) => format!(" · {c}→{t}"),
        (None, Some(t)) => format!(" · →{t}"),
        _ => String::new(),
    };
    let place = if s.local {
        Span::styled("● local", theme::good())
    } else {
        Span::styled("▲ remote", theme::warn())
    };
    let right = Line::from(vec![
        place,
        Span::styled(
            format!(" · {}{levels} · {} logs", s.model, s.entries),
            theme::dim(),
        ),
    ]);
    let room = (width as usize).saturating_sub(right.width() + 2);
    (Line::styled(truncate(left, room), theme::dim()), right)
}

pub fn draw(frame: &mut Frame, app: &App) {
    let area = frame.area();
    let inner_w = area.width.saturating_sub(2);
    let panel = panel_lines(app, inner_w);
    let composer_h = composer_height(app);
    let input_panel = matches!(app.panel, Some(Panel::Input { .. }));
    let [status, panel_area, composer, footer_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(0),
        Constraint::Length(composer_h),
        Constraint::Length(1),
    ])
    .areas(area);

    frame.render_widget(Paragraph::new(status_line(app, area.width)), status);

    let panel_rect = Rect {
        x: panel_area.x + 1,
        width: panel_area.width.saturating_sub(2),
        ..panel_area
    };
    let skip = panel.len().saturating_sub(panel_rect.height as usize);
    frame.render_widget(
        Paragraph::new(panel.into_iter().skip(skip).collect::<Vec<_>>()),
        panel_rect,
    );

    let focused = app.panel.is_none() || input_panel;
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(if focused {
            theme::accent()
        } else {
            theme::dim()
        });
    let inner = block.inner(composer);
    block.render(composer, frame.buffer_mut());
    let [prompt, text] =
        Layout::horizontal([Constraint::Length(2), Constraint::Min(1)]).areas(inner);
    frame.render_widget(
        Paragraph::new(Span::styled("›", theme::accent_bold())),
        prompt,
    );
    match &app.panel {
        Some(Panel::Input { input, .. }) => frame.render_widget(input.as_ref(), text),
        Some(_) => frame.render_widget(
            Paragraph::new(app.composer_text())
                .style(theme::dim())
                .wrap(Wrap { trim: false }),
            text,
        ),
        None => frame.render_widget(&app.composer, text),
    }

    let (left, right) = footer(app, area.width);
    let footer_rect = Rect {
        x: footer_area.x + 1,
        width: footer_area.width.saturating_sub(2),
        ..footer_area
    };
    frame.render_widget(Paragraph::new(left), footer_rect);
    frame.render_widget(Paragraph::new(right).right_aligned(), footer_rect);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Paths;
    use crate::session::Session;
    use ratatui::backend::TestBackend;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::Terminal;

    fn app() -> (App, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let mut session = Session::at(Paths::at(dir.path().to_path_buf())).unwrap();
        // A closed port: no test reaches a real model.
        session.cfg.llm.base_url = "http://127.0.0.1:9".into();
        session.save_config().unwrap();
        (App::new(session, 79, false), dir)
    }

    fn render(app: &App) -> String {
        let h = desired_height(app, 80, 40);
        let mut term = Terminal::new(TestBackend::new(80, h)).unwrap();
        term.draw(|f| draw(f, app)).unwrap();
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

    fn key(app: &mut App, code: KeyCode) {
        app.on_key(KeyEvent::new(code, KeyModifiers::NONE));
    }

    #[test]
    fn people_notes_goals_and_mentions() {
        use crate::people::{NoteKind, Person, Relation};
        let (mut app, _dir) = app();
        app.session
            .add_person(Person {
                handle: "ada".into(),
                name: "Ada".into(),
                role: Some("Junior developer".into()),
                team: None,
                relation: Relation::Mentee,
                about: None,
                since: None,
            })
            .unwrap();

        // @ completes people outside file commands.
        for c in "Paired with @a".chars() {
            key(&mut app, KeyCode::Char(c));
        }
        assert!(render(&app).contains("@ada"), "people popup");
        // Redraws use the cached people; the file is read again only after it changes.
        let reads = app.people_reads.get();
        for _ in 0..20 {
            render(&app);
        }
        assert_eq!(
            app.people_reads.get(),
            reads,
            "no reads while nothing changes"
        );
        key(&mut app, KeyCode::Tab);
        assert_eq!(app.composer_text(), "Paired with @ada ");
        key(&mut app, KeyCode::Esc);
        assert_eq!(app.composer_text(), "");

        // The same rule and message as in the browser.
        app.submit(&format!(
            "/note @ada {}",
            "x".repeat(crate::limits::TEXT + 1)
        ));
        let printed: Vec<String> = app.out.iter().map(markdown::plain).collect();
        assert!(
            printed.iter().any(|l| l.contains(crate::limits::NOTE_LONG)),
            "{printed:?}"
        );
        assert!(app.session.notes().unwrap().is_empty());

        app.submit("/note @ada 1:1 Talked about her first on-call week");
        let notes = app.session.notes().unwrap();
        assert_eq!((notes.len(), notes[0].kind), (1, NoteKind::OneOnOne));
        assert_eq!(notes[0].text, "Talked about her first on-call week");
        app.submit("/undo");
        assert!(app.session.notes().unwrap().is_empty());

        app.submit("/goal Speak at a meetup");
        app.submit("/checkin 1 Sent the proposal");
        assert_eq!(
            app.session.goals().unwrap().get(1).unwrap().checkins.len(),
            1
        );
        app.submit("/undo");
        assert!(app
            .session
            .goals()
            .unwrap()
            .get(1)
            .unwrap()
            .checkins
            .is_empty());
        app.submit("/checkin 1 Wrote the abstract");
        app.submit("/goal show 1");
        let printed: Vec<String> = app.out.iter().map(markdown::plain).collect();
        assert!(
            printed
                .iter()
                .any(|l| l.contains(" 1. ") && l.contains("Wrote the abstract")),
            "{printed:?}"
        );
        app.submit("/checkin delete 1 1");
        let checkins = |app: &App| {
            app.session
                .goals()
                .unwrap()
                .get(1)
                .unwrap()
                .checkins
                .clone()
        };
        assert!(checkins(&app).is_empty());
        app.submit("/undo");
        assert_eq!(checkins(&app)[0].text, "Wrote the abstract");

        // Unclear text that mentions someone offers a note about them.
        app.panel = Some(Panel::Choice("@ada seemed unsure about the rollout".into()));
        assert!(render(&app).contains("note about @ada"));
        key(&mut app, KeyCode::Char('n'));
        assert_eq!(app.session.notes().unwrap()[0].person, "ada");

        // Follow-ups close by the short id lists show, and /undo takes it back.
        app.submit("/note @ada followup Share the retry doc");
        let id = app
            .session
            .notes()
            .unwrap()
            .last()
            .unwrap()
            .short_id()
            .to_string();
        app.submit(&format!("/notes done {}", &id[..4]));
        assert!(!app
            .session
            .notes()
            .unwrap()
            .last()
            .unwrap()
            .is_open_follow_up());
        app.submit("/undo");
        assert!(app
            .session
            .notes()
            .unwrap()
            .last()
            .unwrap()
            .is_open_follow_up());
        app.submit(&format!("/notes delete {id}"));
        assert_eq!(app.session.notes().unwrap().len(), 1);
        app.submit("/undo");
        assert_eq!(app.session.notes().unwrap().last().unwrap().short_id(), id);

        // People can be added without leaving the app.
        let reads = app.people_reads.get();
        app.submit("/people add @bo Bo, Staff engineer, peer");
        app.panel = Some(Panel::Choice("@bo said the rollout can wait".into()));
        assert!(
            render(&app).contains("note about @bo"),
            "the change is seen"
        );
        assert_eq!(app.people_reads.get(), reads + 1);
        app.panel = None;
        let bo = app.session.people().unwrap().get("bo").cloned().unwrap();
        assert_eq!(bo.label(), "Bo (Staff engineer, peer)");
        // A goal whose last word is a ladder expectation is tied to it.
        let ladder =
            crate::ladder::Ladder::from_yaml(include_str!("../../ladder.example.yaml")).unwrap();
        app.session.save_ladder(&ladder).unwrap();
        app.submit("/goal Mentor a junior developer L3.mentoring.1");
        let goals = app.session.goals().unwrap();
        let tied = goals.goals.last().unwrap();
        assert_eq!(
            (tied.text.as_str(), tied.expectation.as_deref()),
            ("Mentor a junior developer", Some("L3.mentoring.1"))
        );

        // Everything can be changed in the app, and /undo takes it back.
        app.submit("/people edit @bo role: Principal engineer, team: Payments, relation: manager");
        let label = |app: &App| app.session.people().unwrap().get("bo").unwrap().label();
        assert_eq!(label(&app), "Bo (Principal engineer, manager)");
        app.submit("/undo");
        assert_eq!(label(&app), "Bo (Staff engineer, peer)");
        app.submit("/note @bo followup Ask about the on-call swap");
        app.submit("/people remove @bo");
        assert!(app.session.people().unwrap().get("bo").is_none());
        assert!(app
            .session
            .notes()
            .unwrap()
            .iter()
            .all(|n| n.person != "bo"));
        app.submit("/undo");
        assert!(app.session.people().unwrap().get("bo").is_some());
        assert!(app
            .session
            .notes()
            .unwrap()
            .iter()
            .any(|n| n.person == "bo"));

        let tied_id = app.session.goals().unwrap().goals.last().unwrap().id;
        app.submit(&format!(
            "/goal edit {tied_id} text: Mentor two developers, due: 2099-12-31, expectation:"
        ));
        let goal = |app: &App| app.session.goals().unwrap().get(tied_id).cloned().unwrap();
        assert_eq!(
            (
                goal(&app).text,
                goal(&app).due.map(|d| d.to_string()),
                goal(&app).expectation
            ),
            (
                "Mentor two developers".to_string(),
                Some("2099-12-31".to_string()),
                None
            )
        );
        app.submit("/undo");
        assert_eq!(goal(&app).expectation.as_deref(), Some("L3.mentoring.1"));

        let note_id = app.session.notes().unwrap()[0].short_id().to_string();
        app.submit(&format!(
            "/notes edit {note_id} given Her reviews are clearer"
        ));
        let first = |app: &App| app.session.notes().unwrap()[0].clone();
        assert_eq!(
            (first(&app).kind, first(&app).text.as_str()),
            (NoteKind::FeedbackGiven, "Her reviews are clearer")
        );
        app.submit("/undo");
        assert_eq!(first(&app).kind, NoteKind::Note);

        app.submit("/people @ada");
        let printed: Vec<String> = app.out.iter().map(markdown::plain).collect();
        assert!(
            printed
                .iter()
                .any(|l| l.contains("Ada (Junior developer, mentee)")),
            "{printed:?}"
        );
    }

    #[test]
    fn composer_popup_and_commands() {
        let (mut app, _dir) = app();
        assert!(render(&app).contains("type / for commands"));

        key(&mut app, KeyCode::Char('/'));
        key(&mut app, KeyCode::Char('g'));
        let s = render(&app);
        assert!(s.contains("/gap") && s.contains("compare your logs"), "{s}");

        // Esc closes the popup, a second Esc clears the input.
        key(&mut app, KeyCode::Esc);
        assert!(!render(&app).contains("compare your logs"));
        key(&mut app, KeyCode::Esc);
        assert_eq!(app.composer_text(), "");

        for c in "/log shipped the export".chars() {
            key(&mut app, KeyCode::Char(c));
        }
        // Typing a space after the command name closes the popup; Enter runs it.
        key(&mut app, KeyCode::Enter);
        assert_eq!(app.session.entries().unwrap().len(), 1);
        let printed: Vec<String> = app.out.iter().map(markdown::plain).collect();
        assert!(
            printed
                .iter()
                .any(|l| l.contains("Logged: shipped the export")),
            "{printed:?}"
        );

        app.submit("/undo");
        assert!(app.session.entries().unwrap().is_empty());

        key(&mut app, KeyCode::Char('?'));
        assert!(render(&app).contains("Commands"));
        key(&mut app, KeyCode::Char('x'));
        assert!(app.panel.is_none());
    }

    #[test]
    fn import_remove_and_undo() {
        use crate::store::Entry;
        let (mut app, dir) = app();
        let day = chrono::NaiveDate::from_ymd_opt(2025, 3, 4).unwrap();
        app.session
            .store
            .append(&[
                Entry::new(day, "Fixed the retry bug", vec![], "import:old.xlsx:Log:R2"),
                Entry::new(
                    day,
                    "Wrote the queue runbook",
                    vec![],
                    "import:old.xlsx:Log:R3",
                ),
                Entry::new(day, "Paired on the cache warmup", vec![], "manual"),
            ])
            .unwrap();

        // The picker lists what was imported; choosing one removes its entries.
        app.submit("/import remove");
        assert!(render(&app).contains("old.xlsx"));
        key(&mut app, KeyCode::Enter);
        let left = app.session.entries().unwrap();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].source, "manual");
        assert!(dir.path().join("staging").read_dir().unwrap().count() == 1);

        app.submit("/undo");
        assert_eq!(app.session.entries().unwrap().len(), 3);

        // By name, and an import applied here is taken back by /undo.
        app.submit("/import remove ~/Downloads/old.xlsx");
        assert_eq!(app.session.entries().unwrap().len(), 1);
        let backup = dir
            .path()
            .join("staging")
            .read_dir()
            .unwrap()
            .next()
            .unwrap();
        let preview = app
            .session
            .import_preview(
                &backup.unwrap().path(),
                None,
                None,
                None,
                &Default::default(),
                &mut crate::no_progress,
            )
            .unwrap();
        app.panel = Some(Panel::Import {
            preview: Box::new(preview),
            scroll: 0,
        });
        key(&mut app, KeyCode::Enter);
        assert_eq!(app.session.entries().unwrap().len(), 3);
        app.submit("/undo");
        assert_eq!(app.session.entries().unwrap().len(), 1);

        app.submit("/import remove nothing.csv");
        let printed: Vec<String> = app.out.iter().map(markdown::plain).collect();
        assert!(
            printed
                .iter()
                .any(|l| l.contains("Nothing was imported from nothing.csv")),
            "{printed:?}"
        );
    }

    fn workbook(path: &std::path::Path, sheets: &[(&str, &[&[&str]])]) {
        let mut book = rust_xlsxwriter::Workbook::new();
        for (name, rows) in sheets {
            let sheet = book.add_worksheet();
            sheet.set_name(*name).unwrap();
            for (r, row) in rows.iter().enumerate() {
                for (c, v) in row.iter().enumerate() {
                    sheet.write_string(r as u32, c as u16, *v).unwrap();
                }
            }
        }
        book.save(path).unwrap();
    }

    #[test]
    fn workbooks_ask_which_sheet_and_where_it_goes() {
        use super::super::app::Purpose;
        use crate::import::workbook::SheetKind;
        let (mut app, dir) = app();

        // A ladder workbook: each sheet gets a role, suggested and changed with ← →.
        let framework = dir.path().join("framework.xlsx");
        workbook(
            &framework,
            &[
                ("Roadmap", &[&["Q3", "Yeni ödeme altyapısı"]]),
                (
                    "Expectations",
                    &[
                        &["L1"],
                        &["- Completes small, well-defined tasks with help"],
                        &["L2"],
                        &["- Delivers features end to end on their own"],
                    ],
                ),
                ("Verbs", &[&["L1", "learns"], &["L2", "delivers"]]),
            ],
        );
        app.submit(&format!("/ladder import {}", framework.display()));
        let screen = render(&app);
        assert!(screen.contains("What is each sheet for?"), "{screen}");
        assert!(screen.contains("L1, L2 · 2 expectations"), "{screen}");
        assert!(screen.contains("verbs per level"), "{screen}");
        // Every sheet skipped: nothing to read.
        key(&mut app, KeyCode::Down);
        key(&mut app, KeyCode::Right);
        key(&mut app, KeyCode::Right);
        key(&mut app, KeyCode::Right);
        key(&mut app, KeyCode::Right);
        assert!(
            render(&app).contains("Expectations  skip"),
            "{}",
            render(&app)
        );
        key(&mut app, KeyCode::Right);
        // Reading it (with a closed model endpoint here).
        key(&mut app, KeyCode::Enter);
        assert_eq!(
            app.job.as_ref().map(|j| j.label.as_str()),
            Some("Reading framework.xlsx")
        );
        while app.job.is_some() {
            std::thread::sleep(std::time::Duration::from_millis(20));
            app.poll_jobs();
        }
        assert!(
            render(&app).contains("Ladder from"),
            "the ladder preview opens"
        );
        key(&mut app, KeyCode::Esc);

        // A 1:1 workbook: each sheet gets a place, changed with ← →.
        let file = dir.path().join("lead.xlsx");
        workbook(
            &file,
            &[
                (
                    "Agenda",
                    &[
                        &["Tarih", "Gündem", "Aksiyon"],
                        &["02.09.2026", "On-call", "Runbook yaz"],
                    ],
                ),
                (
                    "Kariyer",
                    &[&["Beklenti", "Ne yapmalıyım"], &["Mentorluk", "Buddy ol"]],
                ),
                (
                    "Linkler",
                    &[&["Ad", "Link"], &["Wiki", "https://wiki.example.com"]],
                ),
            ],
        );
        app.submit(&format!("/import {}", file.display()));
        let screen = render(&app);
        assert!(screen.contains("Where should each sheet go?"), "{screen}");
        assert!(screen.contains("notes about a person"), "{screen}");
        assert!(
            render(&app).contains("skip"),
            "a sheet without dates is skipped"
        );
        key(&mut app, KeyCode::Down);
        key(&mut app, KeyCode::Down);
        key(&mut app, KeyCode::Right);
        assert!(render(&app).contains("log entries"));
        key(&mut app, KeyCode::Left);
        key(&mut app, KeyCode::Enter);
        match &app.panel {
            Some(Panel::Input {
                purpose: Purpose::ImportPerson(_, kinds),
                ..
            }) => assert_eq!(
                kinds,
                &vec![
                    ("Agenda".to_string(), SheetKind::Notes),
                    ("Kariyer".to_string(), SheetKind::Goals),
                    ("Linkler".to_string(), SheetKind::Skip),
                ]
            ),
            _ => panic!("asks whose notes these are"),
        }
        assert!(render(&app).contains("Whose notes are these?"));
    }

    #[test]
    fn multiline_input_grows_the_area() {
        let (mut app, _dir) = app();
        let one = desired_height(&app, 80, 40);
        app.on_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
        app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT));
        app.on_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::NONE));
        assert_eq!(app.composer_text(), "a\nb");
        assert_eq!(desired_height(&app, 80, 40), one + 1);
        // Never taller than the screen.
        assert!(desired_height(&app, 80, 6) <= 5);
    }
}
