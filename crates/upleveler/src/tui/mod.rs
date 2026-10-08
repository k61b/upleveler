//! Interactive terminal app, in the style of Claude Code / Codex: finished output
//! goes into the terminal's own scrollback, and a small live area at the bottom
//! holds the spinner, popups and the input box.

pub mod app;
pub mod commands;
pub mod complete;
pub mod dashboard;
pub mod history;
pub mod jobs;
pub mod markdown;
pub mod theme;
pub mod ui;

use crate::session::Session;
use anyhow::{bail, Context, Result};
use app::{App, Request};
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::cursor::{MoveTo, Show};
use ratatui::crossterm::event::{
    self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyEventKind,
    KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{
    self, disable_raw_mode, enable_raw_mode, Clear, ClearType, EnterAlternateScreen,
    LeaveAlternateScreen,
};
use ratatui::text::Line;
use ratatui::widgets::{Paragraph, Widget};
use ratatui::{Terminal, TerminalOptions, Viewport};
use std::io::{self, Stdout};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

static ENHANCED_KEYS: AtomicBool = AtomicBool::new(false);

fn enter() -> Result<()> {
    enable_raw_mode()?;
    execute!(io::stdout(), EnableBracketedPaste)?;
    if matches!(terminal::supports_keyboard_enhancement(), Ok(true)) {
        execute!(
            io::stdout(),
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        )?;
        ENHANCED_KEYS.store(true, Ordering::Relaxed);
    }
    Ok(())
}

fn leave() {
    if ENHANCED_KEYS.swap(false, Ordering::Relaxed) {
        let _ = execute!(io::stdout(), PopKeyboardEnhancementFlags);
    }
    let _ = execute!(io::stdout(), DisableBracketedPaste, Show);
    let _ = disable_raw_mode();
}

/// The inline live area. Ratatui's inline viewport has a fixed height, so it is
/// rebuilt (at the same spot) whenever the content needs more or less room.
struct Screen {
    terminal: Terminal<CrosstermBackend<Stdout>>,
    height: u16,
}

impl Screen {
    fn new(height: u16) -> Result<Self> {
        let terminal = Terminal::with_options(
            CrosstermBackend::new(io::stdout()),
            TerminalOptions {
                viewport: Viewport::Inline(height),
            },
        )?;
        Ok(Self { terminal, height })
    }

    /// Clears the live area and leaves the cursor where it started.
    fn clear(&mut self) -> Result<()> {
        self.terminal.clear()?;
        Ok(())
    }

    fn set_height(&mut self, height: u16) -> Result<()> {
        if height != self.height {
            self.clear()?;
            *self = Self::new(height)?;
        }
        Ok(())
    }

    fn rebuild(&mut self) -> Result<()> {
        self.clear()?;
        *self = Self::new(self.height)?;
        Ok(())
    }

    fn print(&mut self, lines: Vec<Line<'static>>) -> Result<()> {
        if lines.is_empty() {
            return Ok(());
        }
        let height = lines.len().min(u16::MAX as usize) as u16;
        self.terminal.insert_before(height, |buf| {
            Paragraph::new(lines).render(buf.area, buf);
        })?;
        Ok(())
    }
}

/// Opens the app. `wizard` starts the setup wizard right away.
pub fn run(session: Session, wizard: bool) -> Result<()> {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        leave();
        default_hook(info);
    }));
    enter()?;
    let result = event_loop(session, wizard);
    leave();
    result
}

fn event_loop(session: Session, wizard: bool) -> Result<()> {
    let (width, _) = terminal::size()?;
    if session.configured() && session.cfg.llm.provider != crate::config::Provider::Openai {
        // Typed text is routed by the model; load it while the screen draws.
        jobs::preload(session.cfg.llm.clone());
    }
    let mut app = App::new(session, width.saturating_sub(1), wizard);
    let mut screen = Screen::new(6)?;
    loop {
        app.poll_jobs();
        if let Some(request) = app.request.take() {
            handle(request, &mut app, &mut screen)?;
        }
        let (width, rows) = terminal::size()?;
        app.width = width.saturating_sub(1);
        let lines = std::mem::take(&mut app.out);
        screen.print(lines)?;
        screen.set_height(ui::desired_height(&app, width, rows))?;
        screen.terminal.draw(|f| ui::draw(f, &app))?;
        if app.quit {
            break;
        }
        if event::poll(Duration::from_millis(80))? {
            match event::read()? {
                Event::Key(key) if key.kind != KeyEventKind::Release => app.on_key(key),
                Event::Paste(text) => app.on_paste(&text),
                Event::Resize(..) => screen.rebuild()?,
                _ => {}
            }
        }
    }
    screen.clear()?;
    Ok(())
}

fn handle(request: Request, app: &mut App, screen: &mut Screen) -> Result<()> {
    match request {
        Request::ClearScreen => {
            execute!(
                io::stdout(),
                Clear(ClearType::All),
                Clear(ClearType::Purge),
                MoveTo(0, 0)
            )?;
            *screen = Screen::new(screen.height)?;
            app.refresh_status();
            app.out.extend(history::welcome(&app.status, app.width));
        }
        Request::Dashboard(tab) => {
            screen.clear()?;
            execute!(io::stdout(), EnterAlternateScreen)?;
            let result = (|| -> Result<()> {
                let mut full = Terminal::new(CrosstermBackend::new(io::stdout()))?;
                full.clear()?;
                let mut dash = dashboard::Dashboard::load(&app.session, tab);
                dashboard::run(&mut full, &mut dash)
            })();
            execute!(io::stdout(), LeaveAlternateScreen)?;
            *screen = Screen::new(screen.height)?;
            result?;
        }
        Request::EditStaging(path) => {
            screen.clear()?;
            leave();
            let edited = edit_file(&path);
            enter()?;
            *screen = Screen::new(screen.height)?;
            match edited {
                Ok(()) => app.reimport(path),
                Err(e) => app.notify(format!("{e:#}")),
            }
        }
    }
    Ok(())
}

fn editor() -> String {
    std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .unwrap_or_else(|_| "vi".into())
}

fn edit_file(path: &Path) -> Result<()> {
    let editor = editor();
    let status = std::process::Command::new("sh")
        .arg("-c")
        .arg(format!("{editor} \"$1\""))
        .arg("sh")
        .arg(path)
        .status()
        .with_context(|| format!("running editor {editor}"))?;
    if !status.success() {
        bail!("editor exited with {status}");
    }
    Ok(())
}

/// Opens `$EDITOR` on a temporary file and returns what was written, minus
/// lines starting with `#`.
pub fn edit_in_editor(initial: &str) -> Result<String> {
    let path = std::env::temp_dir().join(format!("upleveler-{}.md", std::process::id()));
    std::fs::write(
        &path,
        format!("{initial}\n# What did you work on? Lines starting with # are ignored.\n"),
    )?;
    let result = edit_file(&path);
    let content = std::fs::read_to_string(&path).unwrap_or_default();
    let _ = std::fs::remove_file(&path);
    result?;
    Ok(content
        .lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string())
}
