//! События терминала: клавиатура, мышь, ресайз и тик автосохранения.

use std::time::Duration;

use crossterm::event::{self, Event as TerminalEvent, KeyEventKind};

use crate::app::App;

/// Как долго ждём событие, прежде чем отработать тик автосохранения.
const POLL: Duration = Duration::from_millis(500);

/// Применить одно событие терминала к приложению. `false` — читатель просит выйти.
pub fn pump(app: &mut App) -> anyhow::Result<bool> {
    if event::poll(POLL)? {
        match event::read()? {
            TerminalEvent::Key(key) if key.kind != KeyEventKind::Release => app.handle_key(key),
            TerminalEvent::Mouse(mouse) => match mouse.kind {
                event::MouseEventKind::ScrollUp => app.handle_wheel(true),
                event::MouseEventKind::ScrollDown => app.handle_wheel(false),
                _ => {}
            },
            TerminalEvent::Resize(width, height) => app.set_size(width, height),
            _ => {}
        }
    } else {
        app.tick()?;
    }
    Ok(!app.should_quit())
}
