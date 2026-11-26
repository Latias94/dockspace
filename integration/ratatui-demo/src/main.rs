//! Interactive terminal entrypoint (plus `--smoke` headless verification mode).

use std::io;
use std::time::Duration;

use dockspace::runtime::SurfacePresentationResult;
use dockspace_ratatui_demo::{RatatuiDockApp, split_status_bar};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind,
};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};

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
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = ratatui::backend::CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_loop(&mut terminal);

    let _ = disable_raw_mode();
    let _ = execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    );
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
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => match key.code {
                    KeyCode::Char('q') => break,
                    KeyCode::Char('m') => app.toggle_modal(),
                    KeyCode::Char('r') => app = RatatuiDockApp::demo()?,
                    KeyCode::Esc => app.cancel_interaction()?,
                    _ => {}
                },
                Event::Mouse(mouse) => app.handle_mouse(mouse)?,
                _ => {}
            }
        }
    }
    Ok(())
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
