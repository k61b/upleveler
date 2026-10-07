//! People (paper): the people you work with, and one person's page with your
//! notes about them, open follow-ups and the entries that mention them.

use super::{layout, view_header, Tab};
use crate::people::{Note, NoteKind, Person, Relation};
use crate::web::data::DashboardData;
use crate::web::illustrations;
use crate::web::ui::{self, Alert, Button, Chip};
use chrono::NaiveDate;
use maud::{html, Markup};

/// The "add a person" form: what was typed (kept after an error) and a message.
#[derive(Default)]
pub struct PersonForm {
    pub handle: String,
    pub name: String,
    pub role: String,
    pub relation: String,
    pub notice: Option<(Alert, String)>,
}

/// The "add a note" form on a person's page.
pub struct NoteForm {
    pub kind: String,
    /// `YYYY-MM-DD`; the form defaults to today.
    pub date: String,
    pub text: String,
    pub notice: Option<(Alert, String)>,
}

impl NoteForm {
    pub fn empty(today: NaiveDate) -> Self {
        Self {
            kind: NoteKind::Note.as_str().into(),
            date: today.to_string(),
            text: String::new(),
            notice: None,
        }
    }
}

pub fn person_path(handle: &str) -> String {
    format!("{}/{handle}", Tab::People.path())
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// The last day you noted something about them or logged work with them.
fn last_seen(data: &DashboardData, handle: &str) -> Option<NaiveDate> {
    let note = data.notes_about(handle).first().map(|n| n.date);
    let entry = data.mentioning(handle).first().map(|e| e.date);
    note.max(entry)
}

fn person_row(data: &DashboardData, p: &Person) -> Markup {
    let notes = data.notes_about(&p.handle).len();
    let open = data
        .notes_about(&p.handle)
        .iter()
        .filter(|n| n.is_open_follow_up())
        .count();
    let shared = data.mentioning(&p.handle).len();
    html! {
        li.person-row {
            a.person-name href=(person_path(&p.handle)) {
                span.mono { "@" (p.handle) }
                " " (p.label())
            }
            p.muted {
                (plural(notes, "note", "notes"))
                " · " (plural(shared, "entry", "entries")) " together"
                @if open > 0 { " · " (plural(open, "open follow-up", "open follow-ups")) }
                @if let Some(day) = last_seen(data, &p.handle) { " · last " span.mono { (day) } }
            }
        }
    }
}

fn relation_options(selected: &str) -> Markup {
    html! {
        @for r in Relation::ALL {
            option value=(r.as_str()) selected[r.as_str() == selected] { (r.as_str()) }
        }
    }
}

fn add_person_form(form: &PersonForm) -> Markup {
    html! {
        form.add-entry method="post" action=(Tab::People.path()) {
            h2.card-title { "Add a person" }
            @if let Some((kind, text)) = &form.notice { (ui::alert(*kind, text)) }
            div.add-entry-row {
                div.field {
                    label.field-label for="person-handle" { "Handle" }
                    input #person-handle.input type="text" name="handle" value=(form.handle)
                        placeholder="ada" autocomplete="off" required;
                }
                div.field.field--grow {
                    label.field-label for="person-name" { "Name" }
                    input #person-name.input type="text" name="name" value=(form.name)
                        placeholder="Ada" autocomplete="off" required;
                }
            }
            div.add-entry-row {
                div.field.field--grow {
                    label.field-label for="person-role" { "Role " span.muted { "(optional)" } }
                    input #person-role.input type="text" name="role" value=(form.role)
                        placeholder="Junior developer" autocomplete="off";
                }
                div.field {
                    label.field-label for="person-relation" { "They are your" }
                    select #person-relation.input name="relation" { (relation_options(if form.relation.is_empty() { "peer" } else { &form.relation })) }
                }
                button.btn.btn--primary type="submit" { "Add" }
            }
        }
    }
}

pub fn people(data: &DashboardData) -> Markup {
    people_with(data, &PersonForm::default())
}

/// The People page with the add form in a given state.
pub fn people_with(data: &DashboardData, form: &PersonForm) -> Markup {
    let open = data.open_follow_ups();
    layout(
        Tab::People.title(),
        Some(Tab::People),
        html! {
            section.view.container {
                (view_header(
                    Tab::People.title(),
                    html! { "The people you work with and your notes about them. Mention someone in a log as " span.mono { "@handle" } "." },
                    (!open.is_empty()).then(|| ui::chip(Chip::Mono, &plural(open.len(), "open follow-up", "open follow-ups"))),
                ))
                div.paper.panel.stack {
                    @if data.people.people.is_empty() {
                        (ui::empty_state(
                            Some(illustrations::no_logs()),
                            "No people yet. Add someone you work with, then mention them in your logs as @handle.",
                            None,
                        ))
                    } @else {
                        ul.person-list { @for p in &data.people.people { (person_row(data, p)) } }
                    }
                    @if !open.is_empty() {
                        section {
                            h2.card-title { "Open follow-ups" }
                            ul.note-list { @for n in &open { (note_item(n, true)) } }
                        }
                    }
                    (add_person_form(form))
                }
            }
        },
    )
}

/// One note; follow-ups get "Done" (or "Open again"), every note gets "Delete".
fn note_item(n: &Note, show_person: bool) -> Markup {
    let base = format!("{}/notes/{}", person_path(&n.person), n.id);
    html! {
        li.note {
            p.note-meta.muted {
                span.mono { (n.date) }
                " · " (n.kind.label())
                @if show_person { " · " a href=(person_path(&n.person)) { "@" (n.person) } }
                @if n.kind == NoteKind::FollowUp && n.done { " · done" }
            }
            p.note-text { (n.text) }
            div.note-actions {
                @if n.kind == NoteKind::FollowUp {
                    form method="post" action=(format!("{base}/done")) {
                        input type="hidden" name="done" value=(if n.done { "false" } else { "true" });
                        @if show_person { input type="hidden" name="back" value=(Tab::People.path()); }
                        button.btn.btn--outline type="submit" { @if n.done { "Open again" } @else { "Done" } }
                    }
                }
                form method="post" action=(format!("{base}/delete")) {
                    @if show_person { input type="hidden" name="back" value=(Tab::People.path()); }
                    button.btn.btn--outline type="submit" { "Delete" }
                }
            }
        }
    }
}

fn add_note_form(p: &Person, form: &NoteForm) -> Markup {
    html! {
        form.add-entry method="post" action=(format!("{}/notes", person_path(&p.handle))) {
            @if let Some((kind, text)) = &form.notice { (ui::alert(*kind, text)) }
            label.field-label for="note-text" { "What do you want to remember about " (p.name) "?" }
            textarea #note-text.input.textarea name="text" rows="3" maxlength="4000" required
                placeholder="Wants to own a service by the end of the year" data-submit-shortcut {
                (form.text)
            }
            div.add-entry-row {
                div.field {
                    label.field-label for="note-kind" { "Kind" }
                    select #note-kind.input name="kind" {
                        @for k in NoteKind::ALL {
                            option value=(k.as_str()) selected[k.as_str() == form.kind] { (k.label()) }
                        }
                    }
                }
                div.field {
                    label.field-label for="note-date" { "Date" }
                    input #note-date.input type="date" name="date" value=(form.date);
                }
                button.btn.btn--primary type="submit" { "Save note" }
            }
        }
    }
}

/// One person's page, or `None` if no one has that handle.
pub fn person(data: &DashboardData, handle: &str, form: &NoteForm) -> Option<Markup> {
    let p = data.person(handle)?;
    let notes = data.notes_about(&p.handle);
    let entries = data.mentioning(&p.handle);
    let mut facts = Vec::new();
    if let Some(team) = &p.team {
        facts.push(format!("Team {team}"));
    }
    if let Some(since) = p.since {
        facts.push(format!("Since {since}"));
    }
    Some(layout(
        &p.name,
        Some(Tab::People),
        html! {
            section.view.container {
                (view_header(
                    &p.name,
                    html! { span.mono { "@" (p.handle) } " · " (p.label()) @for f in &facts { " · " (f) } },
                    Some(html! {
                        form.prep-form method="post" action="/run" {
                            input type="hidden" name="kind" value="prep";
                            input type="hidden" name="period" value=(p.handle);
                            button.btn.btn--outline type="submit" { "Prepare a 1:1" }
                        }
                        (ui::button(Button::Ghost, "All people", Some(Tab::People.path())))
                    }),
                ))
                div.paper.panel.stack {
                    @if let Some(about) = &p.about { p.person-about { (about) } }
                    (add_note_form(p, form))
                    section {
                        h2.card-title { "Notes " span.muted { "(" (notes.len()) ")" } }
                        @if notes.is_empty() {
                            p.muted { "No notes yet. They stay on this computer, in notes.jsonl." }
                        } @else {
                            ul.note-list { @for n in &notes { (note_item(n, false)) } }
                        }
                    }
                    @let preps: Vec<_> = data.reports.iter().filter(|r| r.name.starts_with(&format!("prep-{}-", p.handle))).collect();
                    @if !preps.is_empty() {
                        section {
                            h2.card-title { "1:1 preparations" }
                            div.report-list {
                                @for r in preps.iter().take(5) {
                                    (ui::report_row(&r.name, r.kind.label(), &r.title, r.date))
                                }
                            }
                        }
                    }
                    section {
                        h2.card-title { "Work together " span.muted { "(" (entries.len()) ")" } }
                        @if entries.is_empty() {
                            p.muted { "No entries mention @" (p.handle) " yet." }
                        } @else {
                            ul.log-list { @for e in entries.iter().take(30) { (ui::log_entry(e)) } }
                        }
                    }
                }
            }
        },
    ))
}
