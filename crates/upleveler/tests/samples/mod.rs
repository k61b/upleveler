//! Invented sample files of the kinds people bring to their first setup, and
//! what Upleveler should read from them. Used by the tests, by
//! `cargo run --example sample_files` (to open them, or try them in the app)
//! and by `cargo run --example eval` (to compare models on them).
//!
//! Everything here is made up: Northwind is the classic fictional company,
//! and the people, services, tickets and numbers are invented. The rows of
//! each sheet and the expected results are built from the same tables.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use upleveler::goals::GoalStatus;
use upleveler::people::NoteKind;

pub struct Files {
    /// The company's career framework: guide, levels, verbs, priorities,
    /// competencies, and sheets that are not about levels.
    pub framework: PathBuf,
    /// 1:1s with the lead: meetings, feedback, a development plan and a
    /// "next level" sheet worked on together.
    pub lead: PathBuf,
    /// Notes and feedback with a colleague.
    pub peer: PathBuf,
    /// An old, messy work diary.
    pub diary: PathBuf,
}

/// Writes every sample file into `dir`.
pub fn write_all(dir: &Path) -> Files {
    std::fs::create_dir_all(dir).unwrap();
    let files = Files {
        framework: dir.join("kariyer-cercevesi.xlsx"),
        lead: dir.join("lider-birebir.xlsx"),
        peer: dir.join("ekip-arkadasi.xlsx"),
        diary: dir.join("eski-notlar.md"),
    };
    workbook(&files.framework, &framework());
    workbook(&files.lead, &lead());
    workbook(&files.peer, &peer());
    std::fs::write(&files.diary, DIARY).unwrap();
    files
}

type Row = Vec<String>;
type Sheet = (String, Vec<Row>);

fn row(cells: &[&str]) -> Row {
    cells.iter().map(|c| c.to_string()).collect()
}

fn workbook(path: &Path, sheets: &[Sheet]) {
    let mut book = rust_xlsxwriter::Workbook::new();
    let wrap = rust_xlsxwriter::Format::new().set_text_wrap();
    let bold = rust_xlsxwriter::Format::new().set_bold().set_text_wrap();
    for (name, rows) in sheets {
        let sheet = book.add_worksheet();
        sheet.set_name(name).unwrap();
        for (c, width) in [24, 50, 40, 40, 30, 40, 16].iter().enumerate() {
            sheet.set_column_width(c as u16, *width).unwrap();
        }
        for (r, cells) in rows.iter().enumerate() {
            for (c, value) in cells.iter().enumerate() {
                if value.is_empty() {
                    continue;
                }
                let format = if r == 0 { &bold } else { &wrap };
                sheet
                    .write_string_with_format(r as u32, c as u16, value, format)
                    .unwrap();
            }
        }
    }
    book.save(path).unwrap();
}

// ---- the career framework ----------------------------------------------------

/// A level as the framework's sheets describe it.
pub struct LevelSpec {
    pub id: &'static str,
    /// The name in the "Levels" sheet, the level's title.
    pub name: &'static str,
    pub summary: &'static str,
    pub years: &'static str,
    /// From "Behaviour verbs": one per theme (delivery, people, technical).
    pub verbs: [&'static str; 3],
    /// From "Growth priorities".
    pub focus: &'static [&'static str],
}

pub const LEVELS: &[LevelSpec] = &[
    LevelSpec {
        id: "L1",
        name: "Associate Engineer",
        summary: "Learns the codebase and delivers small tasks with guidance",
        years: "0-2 years",
        verbs: ["completes", "asks", "learns"],
        focus: &[
            "Learn the codebase and tools",
            "Ask for help early",
            "Write tests for every change",
        ],
    },
    LevelSpec {
        id: "L2",
        name: "Engineer",
        summary: "Delivers features on their own and supports teammates",
        years: "2-4 years",
        verbs: ["delivers", "helps", "applies"],
        focus: &[
            "Own features end to end",
            "Give useful code reviews",
            "Keep services healthy",
        ],
    },
    LevelSpec {
        id: "L3",
        name: "Senior Engineer",
        summary: "Owns projects and services and helps others grow",
        years: "4-7 years",
        verbs: ["drives", "mentors", "designs"],
        focus: &[
            "Lead projects of several weeks",
            "Mentor newer engineers",
            "Write design documents",
        ],
    },
    LevelSpec {
        id: "L4",
        name: "Staff Engineer",
        summary: "Leads work across teams and raises the bar for a domain",
        years: "7+ years",
        verbs: ["leads", "grows", "architects"],
        focus: &[
            "Lead work across teams",
            "Make technical debt visible",
            "Grow senior engineers",
        ],
    },
    LevelSpec {
        id: "L5",
        name: "Senior Staff Engineer",
        summary: "Shapes the technical direction of a product area",
        years: "10+ years",
        verbs: ["steers", "develops", "shapes"],
        focus: &[
            "Set direction for a product area",
            "Build communities of practice",
        ],
    },
    LevelSpec {
        id: "L6",
        name: "Principal Engineer",
        summary: "Sets technical direction for the company",
        years: "12+ years",
        verbs: ["sets", "multiplies", "defines"],
        focus: &[
            "Shape company strategy",
            "Represent engineering outside the company",
        ],
    },
];

/// The competency areas and the titles of their expectations, in order.
pub const AREAS: &[(&str, [&str; 3])] = &[
    ("Craft", ["Code quality", "Testing", "Debugging"]),
    ("Delivery", ["Scope", "Estimation", "Shipping"]),
    (
        "Collaboration",
        ["Code review", "Pairing", "Knowledge sharing"],
    ),
    ("Ownership", ["Reliability", "Incidents", "Technical debt"]),
    ("Communication", ["Writing", "Stakeholders", "Feedback"]),
];

/// For each title (in the order of `AREAS`), what it means at L1 … L6. A
/// cell can hold bullets: each is an expectation of its own.
pub const TEXTS: [[&str; 6]; 15] = [
    [
        "Writes clear code for small changes and fixes review comments quickly",
        "Writes readable, well-structured code that teammates can change safely",
        "Sets the quality bar for the team's code and explains the reasoning in reviews",
        "Improves code quality across several teams through shared patterns and libraries",
        "Defines engineering standards that whole product areas adopt",
        "Shapes how the company writes and maintains software over the years",
    ],
    [
        "Adds tests for the code they change",
        "Chooses the right kind of test for each change and keeps tests reliable",
        "Designs the test strategy for the features the team builds",
        "Raises testing practice across teams and removes slow or flaky suites",
        "Builds testing infrastructure that many teams rely on",
        "Sets the long-term direction for quality engineering",
    ],
    [
        "Debugs problems in their own code with help from teammates",
        "Finds the cause of bugs in the services they work on",
        "Leads the debugging of hard production problems across components",
        "Diagnoses problems that span several services and teams",
        "Solves the organisation's hardest technical problems and teaches the approach",
        "Anticipates whole classes of failure before they happen",
    ],
    [
        "Completes small, well-defined tasks with guidance",
        "Delivers features end to end with little guidance",
        "Owns projects of several weeks from design to launch",
        "Leads multi-team projects that take a quarter or more",
        "Drives programmes that change how a product area works",
        "Sets the technical direction of several product areas at once",
    ],
    [
        "Gives estimates for their own tasks and flags when they slip",
        "Breaks down their work and gives reliable estimates",
        "Plans the work of a small group and adjusts the plan as they learn",
        "Plans work across teams and makes trade-offs visible early",
        "Shapes roadmaps with product leaders based on technical risk",
        "Aligns multi-year technical plans with company strategy",
    ],
    [
        "Ships changes through the team's release process",
        "Ships features safely with monitoring and a rollback plan",
        "Improves how the team ships so releases are frequent and boring",
        "Removes delivery bottlenecks shared by several teams",
        "Designs release practices used across the organisation",
        "Sets expectations for how the whole company delivers software",
    ],
    [
        "Asks for reviews early and acts on the feedback",
        "Gives timely, useful code reviews to teammates",
        "Uses reviews to teach and to keep designs consistent",
        "Reviews designs and code across teams for critical changes",
        "Sets review practices that raise quality across many teams",
        "Reviews the company's most critical technical decisions",
    ],
    [
        "Pairs with teammates to learn the codebase",
        "Pairs to unblock teammates and share context",
        "Mentors newer engineers through regular pairing",
        "Grows senior engineers in several teams",
        "Builds mentoring programmes for groups of engineers",
        "Develops the next generation of technical leaders",
    ],
    [
        "Writes down what they learn for the next person",
        "Keeps team documentation accurate and easy to find",
        "Runs knowledge-sharing sessions in the team",
        "Spreads good practices between teams through talks and guides",
        "Builds communities of practice across the organisation",
        "Represents the company's engineering in the industry",
    ],
    [
        "Follows the on-call runbooks and asks for help when stuck",
        "Keeps the services they work on healthy and monitored",
        "Owns a service's reliability and on-call health",
        "Owns the reliability of a domain made of several services",
        "Sets reliability goals for a whole product area",
        "Sets the company's reliability strategy",
    ],
    [
        "Joins incidents to learn and helps with follow-up tasks",
        "Handles incidents in their services and writes clear postmortems",
        "Leads incident response and drives follow-up actions to completion",
        "Leads complex incidents across teams and fixes their root causes",
        "Improves incident practice across the organisation",
        "Makes the company resilient to large-scale failures",
    ],
    [
        "Leaves code a little better than they found it",
        "Raises technical debt they find and fixes it in small steps",
        "Plans technical debt work into the team's roadmap",
        "- Makes the cost of shared technical debt visible to leadership\n- Leads its reduction across teams",
        "Balances product speed and long-term health across an area",
        "Decides where the company invests in long-term technical health",
    ],
    [
        "Writes clear pull request descriptions and commit messages",
        "Writes short design notes for their features",
        "Writes design documents that explain options and trade-offs",
        "Writes proposals that align several teams on one approach",
        "Writes strategy documents that shape a product area",
        "Writes the technical vision that guides the company",
    ],
    [
        "Keeps the team informed about their progress",
        "Explains technical work to product and design partners",
        "Explains technical plans and risks clearly to non-engineers",
        "Builds trust with product leaders across teams",
        "Influences senior leaders on technical investments",
        "Represents engineering in company-level decisions",
    ],
    [
        "Asks for feedback and acts on it",
        "Gives teammates direct, kind feedback",
        "Gives feedback that helps others grow and asks for it in return",
        "Builds a culture of feedback across teams",
        "Coaches leads on giving and receiving feedback",
        "Models open feedback at every level of the company",
    ],
];

/// The sheet the expectations are read from, and how the others are read.
pub const FRAMEWORK_SHEETS: &[(&str, &str)] = &[
    ("How to use", "skip"),
    ("Levels", "levels"),
    ("Behaviour verbs", "verbs"),
    ("Growth priorities", "focus"),
    ("Competencies", "expectations"),
    ("Compensation bands", "skip"),
    ("Roadmap 2027", "skip"),
];

/// An expectation as it should be read: (id, area, title, text).
pub type Expected = (String, String, String, String);

/// The expectations of level `i` (0 = L1), with their ids.
pub fn expectations(i: usize) -> Vec<Expected> {
    let level = LEVELS[i].id;
    let mut out = Vec::new();
    for (a, (area, titles)) in AREAS.iter().enumerate() {
        let mut n = 0;
        for (t, title) in titles.iter().enumerate() {
            for text in TEXTS[a * 3 + t][i].lines() {
                n += 1;
                out.push((
                    format!("{level}.{}.{n}", area.to_lowercase()),
                    area.to_string(),
                    title.to_string(),
                    text.trim_start_matches("- ").to_string(),
                ));
            }
        }
    }
    out
}

/// The id of the expectation with this title at level `id`.
pub fn expectation_id(id: &str, title: &str) -> String {
    let i = LEVELS.iter().position(|l| l.id == id).unwrap();
    expectations(i)
        .into_iter()
        .find(|(_, _, t, _)| t == title)
        .unwrap()
        .0
}

fn framework() -> Vec<Sheet> {
    let mut competencies = vec![row(&["Northwind engineering competencies"])];
    for (i, level) in LEVELS.iter().enumerate() {
        competencies.push(row(&[level.id]));
        if i > 0 {
            competencies.push(row(&[&format!("Everything in {}, and:", LEVELS[i - 1].id)]));
        }
        for (a, (area, titles)) in AREAS.iter().enumerate() {
            competencies.push(row(&[&format!("{area}:")]));
            for (t, title) in titles.iter().enumerate() {
                competencies.push(row(&[title, TEXTS[a * 3 + t][i]]));
            }
        }
    }
    let mut levels = vec![row(&["Level", "Title", "Summary", "Typical experience"])];
    levels.extend(
        LEVELS
            .iter()
            .map(|l| row(&[l.id, l.name, l.summary, l.years])),
    );
    let mut verbs = vec![row(&["Theme", "L1", "L2", "L3", "L4", "L5", "L6"])];
    for (t, theme) in ["Delivery", "People", "Technical"].iter().enumerate() {
        let mut cells = vec![theme.to_string()];
        cells.extend(LEVELS.iter().map(|l| l.verbs[t].to_string()));
        verbs.push(cells);
    }
    let priorities: Vec<Row> = LEVELS
        .iter()
        .map(|l| {
            let mut cells = vec![l.id.to_string()];
            cells.extend(l.focus.iter().map(|f| f.to_string()));
            cells
        })
        .collect();
    vec![
        (
            "How to use".into(),
            vec![
                row(&["How to use this framework"]),
                row(&["Each level includes everything in the levels below it."]),
                row(&["Use the competencies in 1:1s and promotion discussions, not as a checklist."]),
                row(&["L1 to L6 are the individual contributor track; management levels are described elsewhere."]),
            ],
        ),
        ("Levels".into(), levels),
        ("Behaviour verbs".into(), verbs),
        ("Growth priorities".into(), priorities),
        ("Competencies".into(), competencies),
        (
            "Compensation bands".into(),
            vec![
                row(&["Level", "Band", "Review cycle"]),
                row(&["L1", "A", "Yearly"]),
                row(&["L2", "B", "Yearly"]),
                row(&["L3", "C", "Yearly"]),
            ],
        ),
        (
            "Roadmap 2027".into(),
            vec![
                row(&["Quarter", "Q1", "Q2", "Q3", "Q4"]),
                row(&["Platform", "New build system", "Service mesh", "Cost review", "Hardening"]),
            ],
        ),
    ]
}

// ---- 1:1s with the lead ----------------------------------------------------

/// The lead's handle, as given when importing.
pub const LEAD: &str = "selin";
/// The level the 1:1 file works towards (and the one before it).
pub const CURRENT: &str = "L3";
pub const TARGET: &str = "L4";

/// Where each sheet of the lead workbook should go.
pub const LEAD_SHEETS: &[(&str, &str)] = &[
    ("Toplantılar", "notes"),
    ("Geri bildirim", "notes"),
    ("Gelişim planı", "goals"),
    ("L3 → L4", "goals"),
    ("Linkler", "skip"),
];

/// A meeting row: date (empty = same meeting as above), agenda, notes,
/// actions (bullets), owner, the lead's comment, status.
type Meeting = (
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
);

const MEETINGS: &[Meeting] = &[
    (
        "03.08.2026",
        "Tanışma ve beklentiler",
        "L4 için neye odaklanmam gerektiğini konuştuk",
        "- L3 → L4 sekmesini doldur\n- Son iki çeyreğin işlerini listele",
        "Ben",
        "",
        "Tamam",
    ),
    (
        "",
        "On-call",
        "Geçen hafta üç gece alarm geldi",
        "Alarm eşiklerini birlikte gözden geçir",
        "Selin",
        "Alarm yorgunluğunu fark etmen iyi oldu",
        "",
    ),
    (
        "17.08.2026",
        "Kod review süresi",
        "Review'lar ortalama iki gün sürüyor",
        "Review süresini bir ay ölç",
        "Ben",
        "",
        "Devam",
    ),
    (
        "31.08.2026",
        "Tasarım dokümanı",
        "Sipariş servisinin bölünmesi için ilk taslağı paylaştım",
        "- Alternatifleri ekle\n- Mimari toplantısında sun",
        "Ben",
        "Taslak net, trade-off kısmını güçlendir",
        "Devam",
    ),
    (
        "14.09.2026",
        "Mentorluk",
        "Yeni gelen iki stajyerle haftalık eşleşmeye başladım",
        "",
        "",
        "Bunu düzenli tutman L4 için önemli",
        "",
    ),
    (
        "",
        "Incident",
        "Ödeme kuyruğundaki gecikme incident'ini yönettim",
        "Postmortem aksiyonlarını kapat",
        "Ben",
        "",
        "Tamam",
    ),
    (
        "28.09.2026",
        "Paydaş iletişimi",
        "Ürün ekibine teknik riskleri anlattım",
        "Risk listesini her sprint güncelle",
        "Ben",
        "Anlatımın sadeleşti, böyle devam",
        "",
    ),
    (
        "06.10.2026",
        "Gelecek çeyrek",
        "Platform ekibiyle ortak bir proje önerisi var",
        "Proje önerisini birlikte değerlendir",
        "Selin",
        "",
        "",
    ),
];

/// Feedback and notes: date, from, kind, text.
const FEEDBACK: &[(&str, &str, &str, &str)] = &[
    (
        "10.08.2026",
        "Selin",
        "Feedback",
        "Toplantılarda daha erken söz alabilirsin",
    ),
    (
        "24.08.2026",
        "Ben",
        "Feedback",
        "Selin'e haftalık özetlerin çok işe yaradığını söyledim",
    ),
    (
        "21.09.2026",
        "Selin",
        "Not",
        "Q4'te platform ekibine geçiş ihtimali var",
    ),
    (
        "05.10.2026",
        "Selin",
        "Takip",
        "Terfi paketinin taslağını Kasım başında paylaş",
    ),
];

/// The development plan: goal, related competency (by title), due, status.
const PLAN: &[(&str, &str, &str, &str)] = &[
    (
        "Her hafta en az üç review yap ve ilk 24 saatte dön",
        "Code review",
        "31.12.2026",
        "Devam",
    ),
    (
        "Bir platform projesinde teknik liderlik al",
        "Scope",
        "31.03.2027",
        "",
    ),
    ("Haftada iki gün spor yap", "", "", ""),
];

/// The "L3 → L4" sheet: expectation (as copied, shortened or reworded),
/// self-assessment, evidence, plan, the lead's comment, status.
type Step = (
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
);

const NEXT_LEVEL: &[Step] = &[
    (
        "Owns the reliability of a domain made of several services",
        "3",
        "Ödeme ve fatura servislerinin on-call'ını yürütüyorum",
        "İki servis için SLO tanımla",
        "SLO'ları ekiple birlikte belirle",
        "Devam",
    ),
    (
        "Incidents: Leads complex incidents across teams",
        "4",
        "Ödeme kuyruğu incident'i (INC-2291)",
        "",
        "",
        "Tamam",
    ),
    (
        "Writes proposals that align several teams on one approach",
        "2",
        "",
        "Sipariş servisi bölme önerisini iki ekiple hizala",
        "Önce platform ekibinin onayını al",
        "",
    ),
    (
        "Ekipler arasında kod kalitesini artırmak",
        "2",
        "",
        "Paylaşılan bir lint kuralı seti öner",
        "",
        "",
    ),
    (
        "Pairing: Grows senior engineers in several teams",
        "3",
        "İki stajyerle haftalık eşleşme",
        "Bir senior arkadaşa mentorluk teklif et",
        "",
        "Devam",
    ),
];

fn lead() -> Vec<Sheet> {
    let mut meetings = vec![row(&[
        "Tarih",
        "Gündem",
        "Notlar",
        "Aksiyon",
        "Sorumlu",
        "Lider yorumu",
        "Durum",
    ])];
    meetings.extend(
        MEETINGS
            .iter()
            .map(|m| row(&[m.0, m.1, m.2, m.3, m.4, m.5, m.6])),
    );
    let mut feedback = vec![row(&["Tarih", "Kimden", "Tür", "Not"])];
    feedback.extend(FEEDBACK.iter().map(|f| row(&[f.0, f.1, f.2, f.3])));
    let mut plan = vec![row(&["Hedef", "İlgili yetkinlik", "Son tarih", "Durum"])];
    plan.extend(PLAN.iter().map(|p| row(&[p.0, p.1, p.2, p.3])));
    let mut next = vec![row(&[
        "Beklenti",
        "Öz değerlendirme (1-5)",
        "Kanıt",
        "Ne yapacağım",
        "Lider yorumu",
        "Durum",
    ])];
    next.extend(
        NEXT_LEVEL
            .iter()
            .map(|s| row(&[s.0, s.1, s.2, s.3, s.4, s.5])),
    );
    vec![
        ("Toplantılar".into(), meetings),
        ("Geri bildirim".into(), feedback),
        ("Gelişim planı".into(), plan),
        ("L3 → L4".into(), next),
        (
            "Linkler".into(),
            vec![
                row(&["Ad", "Link"]),
                row(&["Takım wiki'si", "https://wiki.example.com/team"]),
                row(&["On-call takvimi", "https://oncall.example.com"]),
            ],
        ),
    ]
}

/// A note as it should be read: (date, kind, text, done).
pub type ExpectedNote = (String, NoteKind, String, bool);

fn iso(date: &str) -> String {
    let p: Vec<&str> = date.split('.').collect();
    format!("{}-{}-{}", p[2], p[1], p[0])
}

/// The notes about the lead, in the order they are read: the meetings, the
/// feedback sheet, then the lead's comments on the "L3 → L4" sheet (dated
/// `today`: that sheet has no dates).
pub fn lead_notes(today: &str) -> Vec<ExpectedNote> {
    let mut out = Vec::new();
    let mut date = String::new();
    for (d, agenda, notes, actions, owner, comment, status) in MEETINGS {
        if !d.is_empty() {
            date = iso(d);
        }
        let main = match (agenda.is_empty(), notes.is_empty()) {
            (false, false) => format!("Gündem: {agenda}\nNotlar: {notes}"),
            (false, true) => agenda.to_string(),
            _ => notes.to_string(),
        };
        out.push((date.clone(), NoteKind::OneOnOne, main, false));
        let whose = if owner.is_empty() || *owner == "Ben" {
            String::new()
        } else {
            format!(" (Sorumlu: {owner})")
        };
        for action in actions.lines().filter(|a| !a.is_empty()) {
            out.push((
                date.clone(),
                NoteKind::FollowUp,
                format!("{}{whose}", action.trim_start_matches("- ")),
                *status == "Tamam",
            ));
        }
        if !comment.is_empty() {
            out.push((
                date.clone(),
                NoteKind::FeedbackReceived,
                comment.to_string(),
                false,
            ));
        }
    }
    for (d, from, kind, text) in FEEDBACK {
        let kind = match *kind {
            "Feedback" if *from == "Ben" => NoteKind::FeedbackGiven,
            "Feedback" => NoteKind::FeedbackReceived,
            "Takip" => NoteKind::FollowUp,
            _ => NoteKind::Note,
        };
        out.push((iso(d), kind, text.to_string(), false));
    }
    for (expectation, _, _, _, comment, _) in NEXT_LEVEL {
        if !comment.is_empty() {
            out.push((
                today.to_string(),
                NoteKind::FeedbackReceived,
                format!("{expectation}: {comment}"),
                false,
            ));
        }
    }
    out
}

/// A goal as it should be read.
pub struct ExpectedGoal {
    pub text: String,
    /// The expectation it is about; `None` when nothing fits.
    pub expectation: Option<String>,
    /// Whether it is tied without a model (the row repeats the expectation
    /// or names its title).
    pub without_model: bool,
    pub status: GoalStatus,
    pub due: Option<String>,
    pub checkin: Option<String>,
}

pub fn lead_goals() -> Vec<ExpectedGoal> {
    let mut out: Vec<ExpectedGoal> = PLAN
        .iter()
        .map(|(goal, competency, due, status)| ExpectedGoal {
            text: goal.to_string(),
            expectation: (!competency.is_empty()).then(|| expectation_id(TARGET, competency)),
            without_model: true,
            status: if *status == "Tamam" {
                GoalStatus::Done
            } else {
                GoalStatus::Active
            },
            due: (!due.is_empty()).then(|| iso(due)),
            checkin: None,
        })
        .collect();
    let target = expectations(LEVELS.iter().position(|l| l.id == TARGET).unwrap());
    for (expectation, rating, evidence, plan, _, status) in NEXT_LEVEL {
        // Copied word for word, or named by its title; one is reworded.
        let exact = target.iter().find(|(_, _, _, text)| text == expectation);
        let titled = target
            .iter()
            .find(|(_, _, title, _)| expectation.starts_with(&format!("{title}:")));
        let (id, without_model) = match (exact, titled) {
            (Some(e), _) | (None, Some(e)) => (e.0.clone(), true),
            (None, None) => (expectation_id(TARGET, "Code quality"), false),
        };
        let mut checkin = vec![format!("Öz değerlendirme (1-5): {rating}")];
        if !evidence.is_empty() {
            checkin.push(format!("Kanıt: {evidence}"));
        }
        out.push(ExpectedGoal {
            text: if plan.is_empty() {
                expectation.to_string()
            } else {
                plan.to_string()
            },
            expectation: Some(id),
            without_model,
            status: if *status == "Tamam" {
                GoalStatus::Done
            } else {
                GoalStatus::Active
            },
            due: None,
            checkin: Some(checkin.join("\n")),
        });
    }
    out
}

// ---- notes with a colleague ------------------------------------------------

/// The colleague's handle.
pub const PEER: &str = "kaan";

const PEER_ROWS: &[(&str, &str, &str, &str)] = &[
    (
        "06.08.2026",
        "Ben",
        "Feedback",
        "Kaan'a test planının çok anlaşılır olduğunu söyledim",
    ),
    (
        "06.08.2026",
        "Kaan",
        "Feedback",
        "PR'larımı daha küçük tutmamı önerdi",
    ),
    (
        "20.08.2026",
        "Ben",
        "Not",
        "Cache katmanını birlikte tasarlıyoruz",
    ),
    (
        "20.08.2026",
        "Kaan",
        "Takip",
        "Cache PR'ına review sözü verdim",
    ),
    (
        "03.09.2026",
        "Kaan",
        "Feedback",
        "On-call devir notlarımın faydalı olduğunu söyledi",
    ),
    (
        "17.09.2026",
        "Ben",
        "Takip",
        "Q4 hedeflerini konuşmak için zaman ayarla",
    ),
    (
        "01.10.2026",
        "Kaan",
        "1:1",
        "Kariyer hedeflerini konuştuk, bir sonraki seviyeye hazırlanıyor",
    ),
];

pub fn peer_notes() -> Vec<ExpectedNote> {
    PEER_ROWS
        .iter()
        .map(|(d, from, kind, text)| {
            let kind = match *kind {
                "Feedback" if *from == "Ben" => NoteKind::FeedbackGiven,
                "Feedback" => NoteKind::FeedbackReceived,
                "Takip" => NoteKind::FollowUp,
                "1:1" => NoteKind::OneOnOne,
                _ => NoteKind::Note,
            };
            (iso(d), kind, text.to_string(), false)
        })
        .collect()
}

fn peer() -> Vec<Sheet> {
    let mut notes = vec![row(&["Tarih", "Kimden", "Tür", "Not"])];
    notes.extend(PEER_ROWS.iter().map(|r| row(&[r.0, r.1, r.2, r.3])));
    vec![("Notlar".into(), notes)]
}

// ---- an old diary ---------------------------------------------------------

/// Facts the log entries must keep word for word: (date, fragment).
pub const DIARY_FACTS: &[(&str, &str)] = &[
    ("2026-07-27", "640ms'den 180ms'ye"),
    ("2026-08-03", "INC-2291"),
    ("2026-08-03", "4 PR review"),
    ("2026-08-10", "RFC-31"),
    ("2026-08-24", "%99.5"),
    ("2026-08-24", "18 dakikadan 11 dakikaya"),
    ("2026-09-07", "5 kişilik"),
    ("2026-09-21", "12 tanesini"),
];

/// Diary entries (by a fragment of their text) and the expectations each
/// is clearly evidence for (at the current or the target level).
pub const DIARY_MAPPINGS: &[(&str, [&str; 2])] = &[
    ("INC-2291", ["L3.ownership.2", "L4.ownership.2"]),
    ("RFC-31", ["L3.communication.1", "L4.communication.1"]),
    (
        "eşli çalıştık",
        ["L3.collaboration.2", "L4.collaboration.2"],
    ),
    (
        "gözlemlenebilirlik eğitimi",
        ["L3.collaboration.3", "L4.collaboration.3"],
    ),
    (
        "teknik risk sunumu",
        ["L3.communication.2", "L4.communication.2"],
    ),
    ("SLO taslağı", ["L3.ownership.1", "L4.ownership.1"]),
    ("4 PR review", ["L3.collaboration.1", "L4.collaboration.1"]),
];

/// How many entries the diary holds (one per piece of work).
pub const DIARY_ENTRIES: usize = 11;

const DIARY: &str = "# Çalışma günlüğü

## 2026-07-27
- Sipariş API'sinde N+1 sorgusunu düzelttim, p95 640ms'den 180ms'ye indi
- Yeni gelen stajyerle eşli çalıştık, ilk PR'ını birlikte açtık
  - özellikle test yazımına odaklandık

## 2026-08-03
- Ödeme kuyruğundaki gecikme incident'ini yönettim, postmortem yazdım (INC-2291)
- 4 PR review

## 2026-08-10
- Sipariş servisini bölme önerisinin ilk taslağını paylaştım (RFC-31)
- Platform ve ödeme ekipleriyle tasarım toplantısı yaptım

## 2026-08-24
- Fatura servisi için SLO taslağı hazırladım: %99.5 erişilebilirlik
- CI süresini 18 dakikadan 11 dakikaya indirdim

## 2026-09-07
- 5 kişilik ekibe gözlemlenebilirlik eğitimi verdim
- izin

## 2026-09-21
- Ürün ekibine çeyreklik teknik risk sunumu yaptım
- Flaky testlerin 12 tanesini düzelttim
";
