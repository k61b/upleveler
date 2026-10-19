//! Logs (paper): every entry, newest first, grouped by day, with a filter.
//! The filter is a plain GET form; `assets/app.js` refreshes `#log-results` as
//! you type.

use super::{import_upload_form, layout, view_header, Tab};
use crate::export::Format;
use crate::import::ImportedFile;
use crate::store::Entry;
use crate::web::data::DashboardData;
use crate::web::illustrations;
use crate::web::ui::{self, Alert, Button};
use maud::{html, Markup};

/// The most entries one page shows; the filter narrows the rest.
const LIMIT: usize = 300;

/// The filtered list on its own, so it can be swapped in place.
pub fn log_results(data: &DashboardData, query: &str, edit: Option<&EditForm>) -> Markup {
    let matched = data.filtered(query);
    let shown = &matched[..matched.len().min(LIMIT)];
    let mut days: Vec<(chrono::NaiveDate, Vec<&Entry>)> = Vec::new();
    for e in shown {
        match days.last_mut() {
            Some((d, list)) if *d == e.date => list.push(e),
            _ => days.push((e.date, vec![e])),
        }
    }
    let query = query.trim();
    html! {
        div #log-results aria-live="polite" {
            p.muted.log-count {
                @if query.is_empty() {
                    (matched.len()) @if matched.len() == 1 { " entry" } @else { " entries" }
                } @else {
                    (matched.len()) " matching “" (query) "”"
                }
            }
            @if matched.is_empty() {
                @if query.is_empty() {
                    (ui::empty_state(Some(illustrations::no_logs()), "Nothing logged yet.", Some(ui::button(Button::Primary, "Add an entry", Some(Tab::Logs.path())))))
                } @else {
                    (ui::empty_state(Some(illustrations::nothing_matches()), "Nothing matches this filter.", Some(ui::button(Button::Ghost, "Clear the filter", Some(Tab::Logs.path())))))
                }
            }
            @for (day, entries) in &days {
                section.log-day {
                    h2.log-date {
                        span.mono { (day) }
                        span.muted { (day.format("%A")) }
                    }
                    ul.log-list { @for e in entries { (log_entry_with_actions(e, edit)) } }
                }
            }
            @if matched.len() > LIMIT {
                p.muted { "Showing the latest " (LIMIT) ". Narrow it down with the filter." }
            }
        }
    }
}

/// The "add an entry" form: what was typed (kept after an error) and a message
/// about the last submission.
pub struct AddForm {
    pub text: String,
    /// `YYYY-MM-DD`; the form defaults to today.
    pub date: String,
    pub tags: String,
    pub notice: Option<(Alert, String)>,
    /// The "edit entry" form currently open with a problem, if any.
    pub edit: Option<EditForm>,
}

impl AddForm {
    pub fn empty(today: chrono::NaiveDate) -> Self {
        Self {
            text: String::new(),
            date: today.to_string(),
            tags: String::new(),
            notice: None,
            edit: None,
        }
    }
}

/// The "edit entry" form for one entry, keeping what was typed after an error.
pub struct EditForm {
    pub id: String,
    pub text: String,
    pub date: String,
    pub tags: String,
    pub notice: Option<(Alert, String)>,
}

impl EditForm {
    fn from_entry(entry: &Entry) -> Self {
        Self {
            id: entry.id.clone(),
            text: entry.text.clone(),
            date: entry.date.to_string(),
            // Ties to expectations are edited with the tags, so saving keeps them.
            tags: entry
                .tags
                .iter()
                .chain(&entry.linked)
                .cloned()
                .collect::<Vec<_>>()
                .join(", "),
            notice: None,
        }
    }
}

/// The "export logs" control: format and an optional period.
pub struct ExportForm {
    pub format: Format,
    pub period: String,
    pub notice: Option<(Alert, String)>,
}

impl Default for ExportForm {
    fn default() -> Self {
        Self {
            format: Format::Md,
            period: String::new(),
            notice: None,
        }
    }
}

fn format_label(format: Format) -> &'static str {
    match format {
        Format::Md => "Markdown",
        Format::Csv => "CSV",
        Format::Xlsx => "Excel",
        Format::Jsonl => "JSON Lines",
    }
}

fn export_form(form: &ExportForm) -> Markup {
    html! {
        form.export-form method="get" action="/export" {
            h3.card-subtitle { "Export your log" }
            @if let Some((kind, text)) = &form.notice { (ui::alert(*kind, text)) }
            div.add-entry-row {
                div.field {
                    label.field-label for="export-format" { "Format" }
                    select #export-format.input name="format" {
                        @for fmt in [Format::Md, Format::Csv, Format::Xlsx, Format::Jsonl] {
                            option value=(fmt.extension()) selected[form.format == fmt] { (format_label(fmt)) }
                        }
                    }
                }
                div.field.field--grow {
                    label.field-label for="export-period" { "Period " span.muted { "(optional)" } }
                    input #export-period.input type="text" name="period" value=(form.period)
                        placeholder="2026-Q3, H2 or 90d" autocomplete="off";
                }
                button.btn.btn--outline type="submit" { "Export" }
            }
        }
    }
}

fn add_form(form: &AddForm) -> Markup {
    html! {
        form.add-entry method="post" action=(Tab::Logs.path()) {
            @if let Some((kind, text)) = &form.notice { (ui::alert(*kind, text)) }
            label.field-label for="entry-text" { "What did you do?" }
            textarea #entry-text.input.textarea name="text" rows="3" maxlength="4000" required
                placeholder="Shipped the circuit breaker for the payments gateway" data-submit-shortcut {
                (form.text)
            }
            div.add-entry-row {
                div.field {
                    label.field-label for="entry-date" { "Date" }
                    input #entry-date.input type="date" name="date" value=(form.date);
                }
                div.field.field--grow {
                    label.field-label for="entry-tags" { "Tags " span.muted { "(optional)" } }
                    input #entry-tags.input type="text" name="tags" value=(form.tags) placeholder="incident, oncall" autocomplete="off";
                }
                button.btn.btn--primary type="submit" { "Log it" }
            }
        }
    }
}

fn edit_form(entry: &Entry, form: &EditForm) -> Markup {
    let field = |name: &str| format!("entry-{}-{name}", entry.id);
    html! {
        form.add-entry method="post" action=(format!("/logs/{}/edit", entry.id)) {
            @if let Some((kind, text)) = &form.notice { (ui::alert(*kind, text)) }
            label.visually-hidden for=(field("text")) { "What did you do?" }
            textarea.input.textarea id=(field("text")) name="text" rows="2" maxlength="4000" required data-submit-shortcut { (form.text) }
            div.add-entry-row {
                div.field {
                    label.field-label for=(field("date")) { "Date" }
                    input.input id=(field("date")) type="date" name="date" value=(form.date);
                }
                div.field.field--grow {
                    label.field-label for=(field("tags")) { "Tags " span.muted { "(optional)" } }
                    input.input id=(field("tags")) type="text" name="tags" value=(form.tags) placeholder="incident, oncall" autocomplete="off";
                }
                button.btn.btn--outline type="submit" { "Save entry" }
            }
        }
    }
}

fn link_host(url: &str) -> &str {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    rest.split(['/', '?', '#']).next().unwrap_or(rest)
}

fn log_entry_content(entry: &Entry) -> Markup {
    html! {
        p.log-text { (entry.text) }
        @if !entry.tags.is_empty() || !entry.links.is_empty() || !entry.linked.is_empty() {
            div.log-meta {
                @for tag in &entry.tags {
                    a.chip.chip--mono href=(format!("/logs?q={}", ui::query_escape(tag))) { "#" (tag) }
                }
                @for id in &entry.linked {
                    a.chip.chip--mono href="/ladder" title="Evidence for this expectation" { "→ " (id) }
                }
                @for link in &entry.links {
                    a.log-link href=(link) target="_blank" rel="noreferrer noopener" { (link_host(link)) " ↗" }
                }
            }
        }
    }
}

fn log_entry_with_actions(entry: &Entry, edit: Option<&EditForm>) -> Markup {
    let is_editing = edit.is_some_and(|e| e.id == entry.id);
    let default = EditForm::from_entry(entry);
    let form = edit.filter(|e| e.id == entry.id).unwrap_or(&default);
    html! {
        li.log-entry {
            (log_entry_content(entry))
            div.log-entry-actions {
                details.log-entry-edit open[is_editing] {
                    summary.btn.btn--outline { "Edit" }
                    (edit_form(entry, form))
                }
                form method="post" action=(format!("/logs/{}/delete", entry.id)) data-confirm="Delete this entry? This cannot be undone." {
                    button.btn.btn--outline type="submit" { "Delete" }
                }
            }
        }
    }
}

/// Where removing the entries imported from `file` is confirmed (GET) and done (POST).
pub const REMOVE_IMPORT: &str = "/logs/imports/remove";

fn import_meta(f: &ImportedFile) -> Markup {
    html! {
        (f.counts()) " · "
        span.mono { (f.first) " → " (f.last) } " · imported "
        span.mono { (f.imported.with_timezone(&chrono::Local).date_naive()) }
    }
}

/// The files entries were imported from, each with a way to take its entries out again.
fn imports(data: &DashboardData, import_notice: Option<&(Alert, String)>) -> Markup {
    let files = data.imported();
    html! {
        div.imports {
            h3.card-subtitle { "Bring in old notes" }
            (import_upload_form(import_notice))
        }
        @if !files.is_empty() {
            details.imports {
                summary { "Imported files " span.muted { "(" (files.len()) ")" } }
                ul.note-list {
                    @for f in &files {
                        li.note {
                            p.note-text.mono { (f.name) }
                            p.note-meta.muted { (import_meta(f)) }
                            div.note-actions {
                                (ui::button(Button::Outline, "Remove this import", Some(&format!("{REMOVE_IMPORT}?file={}", ui::query_escape(&f.name)))))
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Asks before removing every entry imported from `file`; `None` if none was.
pub fn remove_import(data: &DashboardData, file: &str) -> Option<Markup> {
    let files = data.imported();
    let f = files.iter().find(|f| f.name == file)?;
    let count = f.counts();
    Some(layout(
        &format!("Remove {}", f.name),
        Some(Tab::Logs),
        html! {
            section.view.container {
                (view_header(&format!("Remove the import of {}?", f.name), import_meta(f), None))
                div.paper.panel.stack {
                    p {
                        "This removes the " (count) " imported from " span.mono { (f.name) }
                        ". What you added yourself stays. Importing the file again brings them back; removed log entries are also kept in a copy in the staging folder of your data."
                    }
                    form.add-entry-row method="post" action=(REMOVE_IMPORT) {
                        input type="hidden" name="file" value=(f.name);
                        button.btn.btn--danger type="submit" { "Remove " (count) }
                        a.btn.btn--ghost href=(Tab::Logs.path()) { "Cancel" }
                    }
                }
            }
        },
    ))
}

pub fn logs(data: &DashboardData, query: &str) -> Markup {
    logs_with(
        data,
        query,
        &AddForm::empty(data.today),
        &ExportForm::default(),
    )
}

/// The Logs page with the add and export forms in a given state.
pub fn logs_with(data: &DashboardData, query: &str, form: &AddForm, export: &ExportForm) -> Markup {
    logs_with_import(data, query, form, export, None)
}

/// The Logs page with an optional message on the import form.
pub fn logs_with_import(
    data: &DashboardData,
    query: &str,
    form: &AddForm,
    export: &ExportForm,
    import_notice: Option<&(Alert, String)>,
) -> Markup {
    layout(
        Tab::Logs.title(),
        Some(Tab::Logs),
        html! {
            section.view.container {
                (view_header(Tab::Logs.title(), html! { "Everything you have logged, newest first. Add an entry here or in the terminal app." }, None))
                div.paper.panel {
                    (add_form(form))
                    (export_form(export))
                    (imports(data, import_notice))
                    form.filter role="search" method="get" action=(Tab::Logs.path()) data-live="#log-results" {
                        label.visually-hidden for="q" { "Filter logs" }
                        input #q.input type="search" name="q" value=(query) placeholder="Filter by text, tag or date" autocomplete="off";
                        button.btn.btn--outline type="submit" { "Filter" }
                    }
                    (log_results(data, query, form.edit.as_ref()))
                }
            }
        },
    )
}
