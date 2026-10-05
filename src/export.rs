use crate::store::Entry;
use anyhow::Result;
use chrono::Datelike;
use rust_xlsxwriter::{ExcelDateTime, Format as XlsxFormat, Workbook};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Format {
    Md,
    Csv,
    Xlsx,
    Jsonl,
}

impl Format {
    pub fn extension(self) -> &'static str {
        match self {
            Format::Md => "md",
            Format::Csv => "csv",
            Format::Xlsx => "xlsx",
            Format::Jsonl => "jsonl",
        }
    }
}

const COLUMNS: [&str; 7] = [
    "date",
    "text",
    "tags",
    "links",
    "expectations",
    "source",
    "id",
];

/// Renders the text formats; `xlsx` must go through [`write_xlsx`].
pub fn render(entries: &[&Entry], format: Format) -> Result<String> {
    Ok(match format {
        Format::Jsonl => {
            let mut out = String::new();
            for e in entries {
                out.push_str(&serde_json::to_string(e)?);
                out.push('\n');
            }
            out
        }
        Format::Csv => {
            let mut w = csv::Writer::from_writer(Vec::new());
            w.write_record(COLUMNS)?;
            for e in entries {
                w.write_record(row(e))?;
            }
            String::from_utf8(w.into_inner()?)?
        }
        Format::Md => markdown(entries),
        Format::Xlsx => anyhow::bail!("xlsx is a binary format; write it to a file"),
    })
}

fn row(e: &Entry) -> [String; 7] {
    [
        e.date.to_string(),
        e.text.clone(),
        e.tags.join(", "),
        e.links.join(" "),
        e.expectations.join(", "),
        e.source.clone(),
        e.id.clone(),
    ]
}

fn markdown(entries: &[&Entry]) -> String {
    let mut out = String::from("# Work log\n");
    let mut month = None;
    let mut day = None;
    for e in entries {
        let m = (e.date.year(), e.date.month());
        if month != Some(m) {
            out.push_str(&format!("\n## {}-{:02}\n", m.0, m.1));
            month = Some(m);
            day = None;
        }
        if day != Some(e.date) {
            out.push_str(&format!("\n### {}\n\n", e.date));
            day = Some(e.date);
        }
        let tags: String = e.tags.iter().map(|t| format!(" `#{t}`")).collect();
        let text = e.text.replace('\n', "\n  ");
        out.push_str(&format!("- {text}{tags}\n"));
    }
    out
}

pub fn write_xlsx(entries: &[&Entry], path: &Path) -> Result<()> {
    let mut book = Workbook::new();
    let sheet = book.add_worksheet();
    sheet.set_name("Logs")?;
    let bold = XlsxFormat::new().set_bold();
    let date_fmt = XlsxFormat::new().set_num_format("yyyy-mm-dd");
    let wrap = XlsxFormat::new().set_text_wrap();

    for (c, name) in COLUMNS.iter().enumerate() {
        sheet.write_string_with_format(0, c as u16, *name, &bold)?;
    }
    for (i, e) in entries.iter().enumerate() {
        let r = i as u32 + 1;
        let date = ExcelDateTime::from_ymd(
            e.date.year() as u16,
            e.date.month() as u8,
            e.date.day() as u8,
        )?;
        sheet.write_datetime_with_format(r, 0, &date, &date_fmt)?;
        for (c, value) in row(e).into_iter().enumerate().skip(1) {
            if c == 1 {
                sheet.write_string_with_format(r, c as u16, value, &wrap)?;
            } else {
                sheet.write_string(r, c as u16, value)?;
            }
        }
    }
    for (c, width) in [12.0, 80.0, 20.0, 40.0, 28.0, 30.0, 22.0]
        .into_iter()
        .enumerate()
    {
        sheet.set_column_width(c as u16, width)?;
    }
    sheet.set_freeze_panes(1, 0)?;
    sheet.autofilter(0, 0, entries.len() as u32, COLUMNS.len() as u16 - 1)?;
    book.save(path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn entries() -> Vec<Entry> {
        let d = |m, day| NaiveDate::from_ymd_opt(2025, m, day).unwrap();
        vec![
            Entry::new(
                d(9, 30),
                "Fixed \"quoted\", bug",
                vec!["bugfix".into()],
                "manual",
            ),
            Entry::new(d(10, 1), "Led design review", vec![], "manual"),
        ]
    }

    #[test]
    fn markdown_groups_by_month_and_day() {
        let e = entries();
        let refs: Vec<&Entry> = e.iter().collect();
        let md = render(&refs, Format::Md).unwrap();
        assert!(md.contains("## 2025-09\n\n### 2025-09-30\n\n- Fixed \"quoted\", bug `#bugfix`"));
        assert!(md.contains("## 2025-10"));
    }

    #[test]
    fn csv_and_jsonl_and_xlsx() {
        let e = entries();
        let refs: Vec<&Entry> = e.iter().collect();
        let csv = render(&refs, Format::Csv).unwrap();
        assert!(csv.starts_with("date,text,tags"));
        assert!(csv.contains("\"Fixed \"\"quoted\"\", bug\""));
        let jsonl = render(&refs, Format::Jsonl).unwrap();
        assert_eq!(jsonl.lines().count(), 2);

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.xlsx");
        write_xlsx(&refs, &path).unwrap();
        let tables = crate::import::sheet::read_tables(&path).unwrap();
        assert_eq!(tables[0].rows[1].1[0], "2025-09-30");
        assert_eq!(tables[0].rows[2].1[1], "Led design review");
    }
}
