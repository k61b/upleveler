//! Importing a file from the browser: upload form, reading page and preview.

use crate::session::ImportPreview;
use crate::web::ui::{self, Alert};
use crate::web::views::{layout, layout_refreshing, view_header, Tab};
use maud::{html, Markup};

pub const UPLOAD: &str = "/logs/import";
pub const PREVIEW: &str = "/logs/import/preview";
pub const CONFIRM: &str = "/logs/import/confirm";
pub const CANCEL: &str = "/logs/import/cancel";

/// The upload form shown under "Imported files" on the Logs page.
pub fn upload_form(notice: Option<&(Alert, String)>) -> Markup {
    html! {
        form.import-form method="post" action=(UPLOAD) enctype="multipart/form-data" {
            @if let Some((kind, text)) = notice { (ui::alert(*kind, text)) }
            div.add-entry-row {
                div.field.field--grow {
                    label.field-label for="import-file" { "Import a file" }
                    input #import-file.input type="file" name="file" accept=".txt,.md,.csv,.xlsx" required;
                }
                button.btn.btn--outline type="submit" { "Upload" }
            }
            p.form-hint { "txt, md, csv or xlsx, up to 10 MB." }
        }
    }
}

/// Shown while the file is being read with the model.
pub fn reading(name: &str) -> Markup {
    layout_refreshing(
        "Reading import",
        Some(Tab::Logs),
        2,
        html! {
            section.view.container {
                (view_header("Reading the file", html! { "This page refreshes while the file is read." }, None))
                div.paper.panel {
                    p { "Reading " span.mono { (name) } ". This page refreshes automatically." }
                }
            }
        },
    )
}

/// The preview of what would be imported.
pub fn preview(preview: &ImportPreview, id: &str, name: &str) -> Markup {
    let plan = &preview.plan;
    let entries = plan.new.iter().take(10);
    let more = plan.new.len().saturating_sub(10);
    layout(
        "Import preview",
        Some(Tab::Logs),
        html! {
            section.view.container {
                (view_header("Import preview", html! { "Review what would be added from " span.mono { (name) } }, None))
                div.paper.panel.stack {
                    div.preview-counts {
                        (preview_stat(plan.new.len(), "new entry", "new entries"))
                        (preview_stat(plan.duplicates, "duplicate skipped", "duplicates skipped"))
                        (preview_stat(plan.undated.len(), "undated", "undated"))
                    }
                    @if !preview.collected.warnings.is_empty() {
                        div.preview-warnings {
                            @for w in preview.collected.warnings.iter().take(5) {
                                (ui::alert(Alert::Info, w))
                            }
                        }
                    }
                    @if plan.new.is_empty() && preview.notes.is_empty() && preview.goals.is_empty() {
                        p { "Nothing would be imported." }
                    } @else {
                        @if !plan.new.is_empty() {
                            h3 { "Entries" }
                            ul.log-list {
                                @for e in entries {
                                    (ui::log_entry(e))
                                }
                            }
                            @if more > 0 {
                                p.muted { "and " (more) " more" }
                            }
                        }
                        @if !preview.notes.is_empty() {
                            h3 { "Notes" }
                            p { (preview.notes.len()) " notes about @" (preview.person.as_deref().unwrap_or("?")) }
                        }
                        @if !preview.goals.is_empty() {
                            h3 { "Goals" }
                            p { (preview.goals.len()) " goals" }
                        }
                        div.add-entry-row {
                            form method="post" action=(CONFIRM) {
                                input type="hidden" name="id" value=(id);
                                button.btn.btn--primary type="submit" { "Import" }
                            }
                            form method="post" action=(CANCEL) {
                                input type="hidden" name="id" value=(id);
                                button.btn.btn--ghost type="submit" { "Cancel" }
                            }
                        }
                    }
                }
            }
        },
    )
}

fn preview_stat(n: usize, one: &str, many: &str) -> Markup {
    html! {
        div.stat {
            span.stat-value { (n) }
            span.stat-label { (crate::plural(n, one, many)) }
        }
    }
}

/// The file could not be read.
pub fn error_page(name: &str, detail: &str) -> Markup {
    layout(
        "Import failed",
        Some(Tab::Logs),
        html! {
            section.view.container {
                (view_header("Could not read the file", html! { span.mono { (name) } }, None))
                div.paper.panel.stack {
                    (ui::alert(Alert::Error, detail))
                    p { a.btn.btn--ghost href=(Tab::Logs.path()) { "Back to logs" } }
                }
            }
        },
    )
}
