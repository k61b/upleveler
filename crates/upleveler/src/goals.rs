//! Personal goals in `goals.yaml` (edit by hand any time): free ones ("speak at
//! a meetup") and ones tied to an expectation of the ladder ("strong evidence
//! for SD3.mentoring.1 by December"). Progress comes from the log and check-ins.

use crate::analyze::GapSummary;
use crate::store::Entry;
use anyhow::{bail, Context, Result};
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GoalStatus {
    #[default]
    Active,
    Done,
    Dropped,
}

impl GoalStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            GoalStatus::Active => "active",
            GoalStatus::Done => "done",
            GoalStatus::Dropped => "dropped",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Checkin {
    pub date: NaiveDate,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Goal {
    /// Small number used in commands (`goal done 2`) and the `goal-2` log tag.
    pub id: u32,
    pub text: String,
    /// A ladder expectation id this goal is about (`SD3.mentoring.1`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expectation: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due: Option<NaiveDate>,
    #[serde(default)]
    pub status: GoalStatus,
    pub created: NaiveDate,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub checkins: Vec<Checkin>,
}

impl Goal {
    /// The log tag that links an entry to this goal.
    pub fn tag(&self) -> String {
        format!("goal-{}", self.id)
    }

    /// One line for prompts and lists: `#2 Speak at a meetup (due 2026-12-01)`.
    pub fn line(&self) -> String {
        let mut line = format!("#{} {}", self.id, self.text);
        if let Some(exp) = &self.expectation {
            line.push_str(&format!(" [{exp}]"));
        }
        if let Some(due) = self.due {
            line.push_str(&format!(" (due {due})"));
        }
        if self.status != GoalStatus::Active {
            line.push_str(&format!(" ({})", self.status.as_str()));
        }
        line
    }
}

/// How far a goal has come, from the log, the latest gap analysis and check-ins.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Progress {
    /// Entries mapped to the goal's expectation.
    pub evidence: usize,
    /// Entries tagged `goal-<id>`.
    pub tagged: usize,
    /// The newest of those entries.
    pub last: Option<NaiveDate>,
    /// The expectation's rating in the latest gap analysis.
    pub rating: Option<String>,
    pub checkins: usize,
}

pub fn progress(goal: &Goal, entries: &[Entry], gap: Option<&GapSummary>) -> Progress {
    let tag = goal.tag();
    let mut p = Progress {
        checkins: goal.checkins.len(),
        ..Progress::default()
    };
    for e in entries {
        let mapped = goal
            .expectation
            .as_ref()
            .is_some_and(|x| e.expectations.contains(x));
        let tagged = e.tags.contains(&tag);
        p.evidence += usize::from(mapped);
        p.tagged += usize::from(tagged);
        if mapped || tagged {
            p.last = p.last.max(Some(e.date));
        }
    }
    p.rating = goal.expectation.as_ref().and_then(|x| {
        gap.and_then(|g| g.rows.iter().find(|r| &r.id == x).map(|r| r.rating.clone()))
    });
    p
}

/// All goals in `goals.yaml`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Goals {
    #[serde(default)]
    pub goals: Vec<Goal>,
}

impl Goals {
    /// The goals, or none if the file does not exist yet.
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw =
            fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        serde_norway::from_str(&raw).with_context(|| format!("invalid goals in {}", path.display()))
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let header = "# Your goals, used by upleveler. You can edit this file by hand.\n\
                      # Tag a log entry with goal-<id> to count it toward a goal.\n";
        let yaml = format!("{header}{}", serde_norway::to_string(self)?);
        crate::fsio::write_atomic(path, yaml.as_bytes())
    }

    pub fn get(&self, id: u32) -> Option<&Goal> {
        self.goals.iter().find(|g| g.id == id)
    }

    pub fn active(&self) -> impl Iterator<Item = &Goal> {
        self.goals.iter().filter(|g| g.status == GoalStatus::Active)
    }

    /// Adds a goal with the next free id and returns it.
    pub fn add(
        &mut self,
        text: &str,
        expectation: Option<String>,
        due: Option<NaiveDate>,
        today: NaiveDate,
    ) -> Result<&Goal> {
        let text = text.trim();
        if text.is_empty() {
            bail!("a goal needs a description");
        }
        let id = self.goals.iter().map(|g| g.id).max().unwrap_or(0) + 1;
        self.goals.push(Goal {
            id,
            text: text.to_string(),
            expectation,
            due,
            status: GoalStatus::Active,
            created: today,
            checkins: Vec::new(),
        });
        Ok(self.goals.last().expect("just pushed"))
    }

    pub(crate) fn get_mut(&mut self, id: u32) -> Result<&mut Goal> {
        self.goals
            .iter_mut()
            .find(|g| g.id == id)
            .with_context(|| format!("there is no goal #{id} (see `upleveler goal list`)"))
    }

    pub fn set_status(&mut self, id: u32, status: GoalStatus) -> Result<&Goal> {
        let goal = self.get_mut(id)?;
        goal.status = status;
        Ok(goal)
    }

    /// Removes a check-in (the newest one matching `date` and `text`); false if none matched.
    pub fn remove_checkin(&mut self, id: u32, date: NaiveDate, text: &str) -> Result<bool> {
        let goal = self.get_mut(id)?;
        match goal
            .checkins
            .iter()
            .rposition(|c| c.date == date && c.text == text.trim())
        {
            Some(pos) => {
                goal.checkins.remove(pos);
                Ok(true)
            }
            None => Ok(false),
        }
    }

    pub fn checkin(&mut self, id: u32, date: NaiveDate, text: &str) -> Result<&Goal> {
        let text = text.trim();
        if text.is_empty() {
            bail!("a check-in needs a few words about the progress");
        }
        let goal = self.get_mut(id)?;
        goal.checkins.push(Checkin {
            date,
            text: text.to_string(),
        });
        goal.checkins.sort_by_key(|c| c.date);
        Ok(goal)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyze::GapRow;

    fn d(m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, m, day).unwrap()
    }

    #[test]
    fn goals_get_ids_statuses_and_checkins() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("goals.yaml");
        let mut goals = Goals::load(&path).unwrap();
        assert_eq!(
            goals
                .add("Speak at a meetup", None, Some(d(12, 1)), d(10, 1))
                .unwrap()
                .id,
            1
        );
        let mentoring = goals
            .add(
                "Mentor a junior developer",
                Some("SD3.mentoring.1".into()),
                None,
                d(10, 1),
            )
            .unwrap();
        assert_eq!(mentoring.id, 2);
        assert!(goals.add("  ", None, None, d(10, 1)).is_err());
        goals.checkin(1, d(10, 5), "Submitted the talk").unwrap();
        goals.set_status(2, GoalStatus::Done).unwrap();
        assert!(goals.set_status(9, GoalStatus::Done).is_err());
        goals.save(&path).unwrap();
        let loaded = Goals::load(&path).unwrap();
        assert_eq!(loaded, goals);
        assert_eq!(loaded.active().count(), 1);
        assert_eq!(
            loaded.get(1).unwrap().line(),
            "#1 Speak at a meetup (due 2026-12-01)"
        );
    }

    #[test]
    fn progress_counts_mapped_and_tagged_entries() {
        let mut goals = Goals::default();
        goals
            .add("Mentor", Some("SD3.mentoring.1".into()), None, d(9, 1))
            .unwrap();
        let goal = goals.get(1).unwrap().clone();
        let mut mapped = Entry::new(d(9, 10), "Paired with @ada", vec![], "manual");
        mapped.expectations = vec!["SD3.mentoring.1".into()];
        let tagged = Entry::new(
            d(9, 20),
            "Wrote the mentoring plan",
            vec!["goal-1".into()],
            "manual",
        );
        let other = Entry::new(d(9, 25), "Fixed a bug", vec![], "manual");
        let gap = GapSummary {
            date: d(9, 30),
            current: None,
            target: "SD3".into(),
            rows: vec![GapRow {
                id: "SD3.mentoring.1".into(),
                area: "Mentoring".into(),
                text: "Mentors juniors".into(),
                rating: "partial".into(),
                count: 1,
                last: None,
            }],
            overview: String::new(),
            priorities: vec![],
        };
        let p = progress(&goal, &[mapped, tagged, other], Some(&gap));
        assert_eq!((p.evidence, p.tagged, p.last), (1, 1, Some(d(9, 20))));
        assert_eq!(p.rating.as_deref(), Some("partial"));
    }
}
