//! Reports: the list (shell) and the reader (paper). Reports are Markdown the
//! model wrote from your logs; raw HTML inside them is shown as text, never run.

use super::{layout, view_header, Tab};
use crate::web::data::{DashboardData, Report};
use crate::web::ui::{self, Button, Card, Chip};
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

pub fn reports(data: &DashboardData) -> Markup {
    let body = if data.reports.is_empty() {
        ui::card(
            Card::Dashed,
            ui::empty_state(
                None,
                "No reports yet. Gap analyses, promotion documents and summaries appear here after you run them.",
                Some(ui::code_block("upleveler gap")),
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
            section.view.container {
                (view_header(Tab::Reports.title(), html! { "Gap analyses, promotion documents and summaries you have generated." }, None))
                (body)
            }
        },
    )
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
