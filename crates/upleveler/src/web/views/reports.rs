//! Reports: the list (shell) and the reader (paper). Reports are Markdown the
//! model wrote from your logs; raw HTML inside them is shown as text, never run.

use super::{layout, layout_refreshing, view_header, Tab};
use crate::web::data::{DashboardData, Report};
use crate::web::runs::{Kind, RunView, State};
use crate::web::ui::{self, Alert, Button, Card, Chip};
use maud::{html, Markup, PreEscaped};
use pulldown_cmark::{html as md_html, CowStr, Event, Options, Parser};

/// Markdown to HTML with tables and strikethrough; HTML in the source is escaped.
pub fn render_markdown(markdown: &str) -> String {
    let options = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH;
    let events = Parser::new_ext(markdown, options).map(|event| match event {
        Event::Html(raw) | Event::InlineHtml(raw) => Event::Text(CowStr::from(raw.into_string())),
        other => other,
    });
    let mut out = String::new();
    md_html::push_html(&mut out, events);
    out
}

/// The "run an analysis" form: what was chosen (kept after an error) and a
/// message about the last attempt.
pub struct RunForm {
    pub kind: Kind,
    pub period: String,
    pub notice: Option<(Alert, String)>,
}

impl Default for RunForm {
    fn default() -> Self {
        Self {
            kind: Kind::Gap,
            period: String::new(),
            notice: None,
        }
    }
}

fn model_hint(data: &DashboardData) -> String {
    let place = if data.model_local {
        "on this computer"
    } else {
        "at your company's endpoint"
    };
    format!(
        "Uses {} {place}. It can take a few minutes; you can leave the page while it runs.",
        data.model
    )
}

fn run_card(data: &DashboardData, run: Option<&RunView>, form: &RunForm) -> Markup {
    let running = run.filter(|r| r.state == State::Running);
    ui::card(
        Card::Raised,
        html! {
            h2.card-title { "Run an analysis" }
            @if let Some(run) = running {
                p { (run.kind.title()) " for " (run.period) " is running: " (run.step) "." }
                div.actions.run-actions { (ui::button(Button::Inverse, "See progress", Some("/run"))) }
            } @else {
                form.run-form method="post" action="/run" {
                    @if let Some((kind, text)) = &form.notice { (ui::alert(*kind, text)) }
                    div.add-entry-row {
                        div.field {
                            label.field-label for="run-kind" { "Analysis" }
                            select #run-kind.input name="kind" {
                                @for kind in [Kind::Gap, Kind::Brag, Kind::Summary] {
                                    option value=(kind.value()) selected[form.kind == kind] { (kind.title()) }
                                }
                            }
                        }
                        div.field.field--grow {
                            label.field-label for="run-period" { "Period " span.muted { "(optional)" } }
                            input #run-period.input type="text" name="period" value=(form.period)
                                placeholder="2026-Q3, H2 or 90d" autocomplete="off";
                        }
                        button.btn.btn--primary type="submit" { "Start" }
                    }
                    p.hint { (model_hint(data)) " Summaries cover the last 7 days unless you give a period." }
                }
            }
        },
    )
}

pub fn reports(data: &DashboardData) -> Markup {
    reports_with(data, None, &RunForm::default())
}

/// The Reports page with the current analysis (if any) and the run form.
pub fn reports_with(data: &DashboardData, run: Option<&RunView>, form: &RunForm) -> Markup {
    let list = if data.reports.is_empty() {
        ui::card(
            Card::Dashed,
            ui::empty_state(
                None,
                "No reports yet. Gap analyses, promotion documents and summaries appear here after you run them.",
                None,
            ),
        )
    } else {
        ui::card(
            Card::Raised,
            html! {
                div.report-list {
                    @for r in &data.reports { (ui::report_row(&r.name, r.kind.label(), &r.title, r.date)) }
                }
            },
        )
    };
    layout(
        Tab::Reports.title(),
        Some(Tab::Reports),
        html! {
            section.view.container.stack {
                (view_header(Tab::Reports.title(), html! { "Gap analyses, promotion documents and summaries, generated from your logs." }, None))
                (run_card(data, run, form))
                (list)
            }
        },
    )
}

fn duration(seconds: u64) -> String {
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

/// The current (or last) analysis started from the browser. While it runs the
/// page reloads itself every two seconds.
pub fn run(run: Option<&RunView>) -> Markup {
    let Some(run) = run else {
        return layout(
            "Analysis",
            Some(Tab::Reports),
            html! {
                section.view.container {
                    (ui::empty_state(None, "Nothing is running.", Some(ui::button(Button::Ghost, "Back to reports", Some(Tab::Reports.path())))))
                }
            },
        );
    };
    let body = html! {
        section.view.container.reader-wrap {
            (view_header(run.kind.title(), html! { "Period: " (run.period) }, None))
            (ui::card(Card::Raised, html! {
                @match &run.state {
                    State::Running => {
                        p.run-step { (run.step) }
                        @if run.total > 0 {
                            progress.progress value=(run.done) max=(run.total) aria-label="Progress" {}
                            p.muted.run-meta { span.mono { (run.done) " of " (run.total) } " · " span.mono { (duration(run.seconds)) } }
                        } @else {
                            progress.progress aria-label="Progress" {}
                            p.muted.run-meta { span.mono { (duration(run.seconds)) } }
                        }
                        @if run.done == 0 && run.seconds >= 15 {
                            p.hint { "The first step also loads the model, which can take a minute or two on a laptop." }
                        }
                        @if run.stopping {
                            (ui::alert(Alert::Info, "Stopping after the current step. A model call that already started finishes first."))
                        } @else {
                            form method="post" action="/run/cancel" {
                                button.btn.btn--outline type="submit" { "Stop" }
                            }
                        }
                        p.hint { "This page refreshes every 2 seconds. You can close it; the analysis keeps running while upleveler web is open." }
                    },
                    State::Done(name) => {
                        (ui::alert(Alert::Success, &format!("Finished in {}.", duration(run.seconds))))
                        div.actions.run-actions {
                            (ui::button(Button::Primary, "Open the report", Some(&format!("/reports/{name}"))))
                            (ui::button(Button::Ghost, "Back to reports", Some(Tab::Reports.path())))
                        }
                    },
                    State::Failed(message) => {
                        (ui::alert(Alert::Error, &format!("The analysis stopped: {message}")))
                        div.actions.run-actions { (ui::button(Button::Ghost, "Back to reports", Some(Tab::Reports.path()))) }
                    },
                    State::Cancelled => {
                        (ui::alert(Alert::Info, "Stopped. Nothing was saved."))
                        div.actions.run-actions { (ui::button(Button::Ghost, "Back to reports", Some(Tab::Reports.path()))) }
                    },
                }
            }))
        }
    };
    if run.state == State::Running {
        layout_refreshing(run.kind.title(), Some(Tab::Reports), 2, body)
    } else {
        layout(run.kind.title(), Some(Tab::Reports), body)
    }
}

pub fn report(report: &Report) -> Markup {
    layout(
        &report.title,
        Some(Tab::Reports),
        html! {
            section.view.container.reader-wrap {
                div.reader-bar {
                    (ui::button(Button::Ghost, "All reports", Some(Tab::Reports.path())))
                    (ui::copy_button(Button::Outline, "Copy Markdown", &report.markdown))
                }
                article.paper.panel.reader {
                    div.reader-meta {
                        (ui::chip(Chip::Soft, report.kind.label()))
                        (ui::chip(Chip::Mono, &report.date.to_string()))
                        (ui::chip(Chip::Mono, &format!("{}.md", report.name)))
                    }
                    div.prose { (PreEscaped(render_markdown(&report.markdown))) }
                }
            }
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_renders_tables_and_escapes_html() {
        let html = render_markdown(
            "# Title\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\n<script>alert(1)</script> <b>x</b>\n",
        );
        assert!(html.contains("<h1>Title</h1>") && html.contains("<table>"));
        assert!(!html.contains("<script>") && !html.contains("<b>"));
        assert!(html.contains("&lt;script&gt;"));
    }
}
