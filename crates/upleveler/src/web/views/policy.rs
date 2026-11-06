//! The company AI policy page: the policy in force, and writing one with the
//! model's help (question by question, or from the company's written rules).

use crate::policy_draft::Turn;
use crate::web::ui::{self, Alert};
use crate::web::views::{layout, view_header};
use maud::{html, Markup};

pub const PAGE: &str = "/policy";
pub const STEP: &str = "/policy/step";
pub const DOCUMENT: &str = "/policy/document";
pub const SAVE: &str = "/policy/save";
pub const DOWNLOAD: &str = "/policy/download";

/// What the policy page shows.
pub struct PolicyPage {
    /// The policy in force, as `upleveler policy` prints it.
    pub in_force: String,
    /// Whether a model can help (AI on and allowed).
    pub ai: bool,
    pub notice: Option<(Alert, String)>,
}

/// A conversation in progress: what was asked and answered, the question
/// waiting, and the state the next form posts back.
pub struct PolicyChatView<'a> {
    pub turns: &'a [Turn],
    pub question: &'a str,
    pub state: &'a str,
    pub notice: Option<(Alert, String)>,
}

/// A finished draft: the file to review (and edit) before it is saved.
pub struct PolicyResult<'a> {
    pub text: &'a str,
    pub summary: &'a str,
    pub notes: &'a [String],
    /// The commented example rather than a draft: never offered as the
    /// person's own policy.
    pub example: bool,
    pub notice: Option<(Alert, String)>,
}

fn page(title: &str, lead: &str, body: Markup) -> Markup {
    layout(
        title,
        None,
        html! {
            section.view.container {
                (view_header(title, html! { (lead) }, None))
                div.paper.panel.stack { (body) }
            }
        },
    )
}

pub fn policy(view: &PolicyPage) -> Markup {
    page(
        "Company AI policy",
        "What your company allows Upleveler to do with AI, and a way to write the rules down.",
        html! {
            @if let Some((kind, text)) = &view.notice { (ui::alert(*kind, text)) }
            h2.card-title { "In force on this computer" }
            p.policy-in-force { (view.in_force) }
            h2.card-title { "Write a policy" }
            @if view.ai {
                p.muted {
                    "The model asks one question at a time and fills in the file from your answers. "
                    "You read the result before anything is saved."
                }
                form.policy-form method="post" action=(STEP) {
                    input type="hidden" name="state" value="";
                    button.btn.btn--primary type="submit" { "Start with questions" }
                }
                form.policy-form method="post" action=(DOCUMENT) {
                    label.field-label for="policy-rules" { "Or paste your company's written rules about AI use" }
                    textarea #policy-rules.input.textarea name="text" rows="6" maxlength="100000" required
                        placeholder="Employees may use only the approved AI assistant. Notes about colleagues stay on their computer." {}
                    button.btn.btn--outline type="submit" { "Turn them into a policy" }
                }
            } @else {
                p.muted {
                    "Without a model there is nobody to ask, so start from the commented example: "
                    "download it, edit it, and check it with "
                    span.mono { "upleveler policy check policy.toml" } "."
                }
                form.policy-form method="post" action=(DOWNLOAD) {
                    input type="hidden" name="text" value=(crate::policy_draft::TEMPLATE);
                    button.btn.btn--outline type="submit" { "Download the example" }
                }
            }
        },
    )
}

pub fn chat(view: &PolicyChatView) -> Markup {
    page(
        "Write a policy",
        "Answer in your own words. Say \"done\" when you have nothing to add.",
        html! {
            @if let Some((kind, text)) = &view.notice { (ui::alert(*kind, text)) }
            @for turn in view.turns {
                div {
                    p.muted { (turn.question) }
                    p { (turn.answer) }
                }
            }
            form.policy-form method="post" action=(STEP) {
                input type="hidden" name="state" value=(view.state);
                label.field-label for="policy-answer" { (view.question) }
                textarea #policy-answer.input.textarea name="answer" rows="3" maxlength="4000" required autofocus data-submit-shortcut {}
                div.add-entry-row {
                    button.btn.btn--primary type="submit" { "Answer" }
                    a.btn.btn--outline href=(PAGE) { "Stop" }
                }
            }
        },
    )
}

pub fn result(view: &PolicyResult) -> Markup {
    page(
        if view.example {
            "Example policy"
        } else {
            "Your policy"
        },
        "Read it, change anything you like, then save it.",
        html! {
            @if let Some((kind, text)) = &view.notice { (ui::alert(*kind, text)) }
            @if !view.notes.is_empty() {
                div {
                    h2.card-title { "What the rules leave open" }
                    ul { @for n in view.notes { li { (n) } } }
                }
            }
            @if !view.summary.is_empty() { p.muted { (view.summary) } }
            form.policy-form method="post" action=(SAVE) {
                label.field-label for="policy-text" { "policy.toml" }
                textarea #policy-text.input.textarea.mono name="text" rows="24" maxlength="100000" spellcheck="false" { (view.text) }
                div.add-entry-row {
                    @if !view.example {
                        button.btn.btn--primary type="submit" { "Use it on this computer" }
                    }
                    button.btn.btn--outline type="submit" formaction=(DOWNLOAD) { "Download policy.toml" }
                    a.btn.btn--outline href=(PAGE) { "Discard" }
                }
                p.form-hint {
                    "Using it here saves ~/.upleveler/policy.toml; a policy your company installed still applies, and the stricter rule wins. "
                    "To enforce it everywhere, put the file in /Library/Application Support/Upleveler/ (macOS) or /etc/upleveler/ (Linux) on each computer."
                }
            }
        },
    )
}
