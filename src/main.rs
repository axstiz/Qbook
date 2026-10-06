use std::io::{Write, stdout};

use anyhow::Result;
use clap::Parser;
use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use crossterm::execute;

use qbook::app::App;
use qbook::cli::Cli;
use qbook::event;
use qbook::store::Store;
use qbook::ui;

fn main() -> Result<()> {
    let cli = Cli::parse();
    let store = Store::open_default()?;
    let mut app = match cli.path {
        Some(path) => App::load(&path, &cli.lang, &cli.variants, Some(store))?,
        None => App::shelf(Some(store), &cli.lang)?,
    };

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
        terminal.draw(|frame| ui::render(app, frame))?;
        if !event::pump(app)? {
            break;
        }
        if let Some(text) = app.take_clipboard() {
            write!(stdout(), "{}", qbook::clip::osc52(&text))?;
            stdout().flush()?;
        }
    }
    Ok(())
}
