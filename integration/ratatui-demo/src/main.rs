//! Interactive terminal entrypoint (plus `--smoke` headless verification mode).

use std::io::{self, Write as _};
use std::time::Duration;

use dockspace::runtime::SurfacePresentationResult;
use dockspace_ratatui_demo::{RatatuiDockApp, split_status_bar};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
    KeyModifiers,
};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};

struct TerminalCleanupGuard;

impl Drop for TerminalCleanupGuard {
    fn drop(&mut self) {
        let mut stdout = io::stdout();
        let _ = execute!(stdout, DisableMouseCapture, LeaveAlternateScreen);
        let _ = stdout.flush();
        let _ = disable_raw_mode();
    }
}

fn should_quit(key: KeyEvent) -> bool {
    if key.kind == KeyEventKind::Release {
        return false;
    }
    match key.code {
        KeyCode::Char('q' | 'Q') => true,
        KeyCode::Char('c' | 'C' | 'd' | 'D') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            true
        }
        _ => false,
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::args().any(|arg| arg == "--smoke") {
        return run_headless_smoke();
    }
    run_interactive_terminal()
}

fn run_headless_smoke() -> Result<(), Box<dyn std::error::Error>> {
    let mut terminal = Terminal::new(TestBackend::new(100, 30))?;
    let mut app = RatatuiDockApp::demo()?;
    let (dock_area, _) = split_status_bar(terminal.get_frame().area());
    let (display_list, token) = app
        .prepare_frame(dock_area)?
        .expect("initial smoke frame is paintable");
    terminal.draw(|frame| app.draw(frame, &display_list))?;
    app.finish_frame(token, SurfacePresentationResult::Presented)?;
    let rendered = buffer_to_text(terminal.backend().buffer());
    assert!(rendered.contains("editor.rs"));
    assert!(rendered.contains("Cargo.toml"));
    assert!(rendered.contains("Terminal"));
    println!("ratatui-demo headless smoke passed (100x30 buffer rendered).");
    Ok(())
}

fn run_interactive_terminal() -> Result<(), Box<dyn std::error::Error>> {
    enable_raw_mode()?;
    let _cleanup = TerminalCleanupGuard;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = ratatui::backend::CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_loop(&mut terminal);
    let _ = terminal.show_cursor();
    result
}

fn run_loop(
    terminal: &mut Terminal<ratatui::backend::CrosstermBackend<io::Stdout>>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut app = RatatuiDockApp::demo()?;
    loop {
        let size = terminal.size()?;
        let area = ratatui::layout::Rect::new(0, 0, size.width, size.height);
        let (dock_area, _) = split_status_bar(area);
        if let Some((display_list, token)) = app.prepare_frame(dock_area)? {
            let draw_result = terminal.draw(|frame| app.draw(frame, &display_list));
            match draw_result {
                Ok(_) => app.finish_frame(token, SurfacePresentationResult::Presented)?,
                Err(error) => {
                    let _ = app.finish_frame(token, SurfacePresentationResult::Dropped);
                    return Err(error.into());
                }
            }
        }

        if event::poll(Duration::from_millis(50))? {
            loop {
                match event::read()? {
                    Event::Key(key) if should_quit(key) => return Ok(()),
                    Event::Key(key) if key.kind != KeyEventKind::Release => match key.code {
                        KeyCode::Char('m' | 'M') => app.toggle_modal(),
                        KeyCode::Char('r' | 'R') => app = RatatuiDockApp::demo()?,
                        KeyCode::Esc => {
                            if app.is_modal_open() {
                                app.toggle_modal();
                            } else {
                                app.cancel_interaction()?;
                            }
                        }
                        _ => {}
                    },
                    Event::Mouse(mouse) => app.handle_mouse(mouse)?,
                    _ => {}
                }
                if !event::poll(Duration::ZERO)? {
                    break;
                }
            }
        }
    }
}

fn buffer_to_text(buffer: &ratatui::buffer::Buffer) -> String {
    let mut out = String::new();
    for y in 0..buffer.area.height {
        for x in 0..buffer.area.width {
            out.push_str(buffer[(x, y)].symbol());
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quit_shortcuts_accept_q_and_ctrl_c() {
        assert!(should_quit(KeyEvent::new(
            KeyCode::Char('q'),
            KeyModifiers::NONE
        )));
        assert!(should_quit(KeyEvent::new(
            KeyCode::Char('Q'),
            KeyModifiers::SHIFT
        )));
        assert!(should_quit(KeyEvent::new(
            KeyCode::Char('c'),
            KeyModifiers::CONTROL
        )));
        assert!(should_quit(KeyEvent::new(
            KeyCode::Char('d'),
            KeyModifiers::CONTROL
        )));
        assert!(!should_quit(KeyEvent::new(
            KeyCode::Char('c'),
            KeyModifiers::NONE
        )));
    }
}
