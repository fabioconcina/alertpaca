pub mod render;

use std::sync::mpsc;
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::DefaultTerminal;

use crate::checks::{self, CheckResult};
use crate::config::Config;

pub struct App {
    pub results: Vec<CheckResult>,
    pub last_check: Option<chrono::DateTime<chrono::Local>>,
    pub checking: bool,
    pub scroll_offset: u16,
    pub content_height: u16,
    pub config: Config,
}

impl App {
    pub fn new(config: Config) -> Self {
        Self {
            results: Vec::new(),
            last_check: None,
            checking: false,
            scroll_offset: 0,
            content_height: 0,
            config,
        }
    }

    fn scroll_up(&mut self) {
        self.scroll_offset = self.scroll_offset.saturating_sub(1);
    }

    fn scroll_down(&mut self, viewport_height: u16) {
        let max = self.content_height.saturating_sub(viewport_height);
        if self.scroll_offset < max {
            self.scroll_offset += 1;
        }
    }
}

fn trigger_refresh(config: Config) -> mpsc::Receiver<Vec<CheckResult>> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let results = checks::run_all_checks(&config);
        let _ = tx.send(results);
    });
    rx
}

pub fn run(terminal: &mut DefaultTerminal, config: Config) -> Result<()> {
    let mut app = App::new(config);
    let tick_rate = Duration::from_millis(100);
    let refresh_interval = Duration::from_secs(60);

    // Initial check
    let mut pending_rx = Some(trigger_refresh(app.config.clone()));
    app.checking = true;
    let mut last_refresh = Instant::now();

    loop {
        let viewport_height = terminal.draw(|f| render::draw(f, &mut app))?.area.height;

        if event::poll(tick_rate)? {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                match key.code {
                    KeyCode::Char('q') => break,
                    KeyCode::Char('r') => {
                        if !app.checking {
                            pending_rx = Some(trigger_refresh(app.config.clone()));
                            app.checking = true;
                            last_refresh = Instant::now();
                        }
                    }
                    KeyCode::Up | KeyCode::Char('k') => app.scroll_up(),
                    KeyCode::Down | KeyCode::Char('j') => app.scroll_down(viewport_height),
                    _ => {}
                }
            }
        }

        // Check for completed results
        if let Some(ref rx) = pending_rx {
            if let Ok(results) = rx.try_recv() {
                app.results = results;
                app.last_check = Some(chrono::Local::now());
                app.checking = false;
                pending_rx = None;
            }
        }

        // Auto-refresh
        if last_refresh.elapsed() >= refresh_interval && !app.checking {
            pending_rx = Some(trigger_refresh(app.config.clone()));
            app.checking = true;
            last_refresh = Instant::now();
        }
    }

    Ok(())
}
