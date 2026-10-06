use std::io::stdout;

use anyhow::Result;
use clap::Parser;
use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use crossterm::execute;

use qbook::app::App;
use qbook::cli::Cli;
use qbook::event;
use qbook::store::Store;
use qbook::ui::reader;

fn main() -> Result<()> {
    let cli = Cli::parse();
    let mut app = App::load(&cli.path, &cli.lang, &cli.variants, Some(Store::open_default()?))?;

    let mut terminal = ratatui::init();
    execute!(stdout(), EnableMouseCapture)?;
    let result = run(&mut terminal, &mut app);
    execute!(stdout(), DisableMouseCapture)?;
    ratatui::restore();

    let saved = app.save_progress();
    result?;
    saved?;
    Ok(())
}

fn run(terminal: &mut ratatui::DefaultTerminal, app: &mut App) -> Result<()> {
    let (width, height) = crossterm::terminal::size()?;
    app.set_size(width, height);
    loop {
        terminal.draw(|frame| reader::render(app, frame))?;
        if !event::pump(app)? {
            break;
        }
    }
    Ok(())
}
