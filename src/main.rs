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
    let mut app = match (&cli.path, cli.lang.as_deref()) {
        (Some(path), Some(lang)) => App::load(path, lang, &cli.variants, Some(store))?,
        (Some(path), None) => App::load_auto(path, &cli.variants, Some(store))?,
        (None, _) => App::shelf(Some(store), cli.lang.as_deref().unwrap_or("en"))?,
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
