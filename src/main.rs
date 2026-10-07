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
    let config = qbook::config::load(cli.theme.as_deref())?;
    let store = Store::open_default()?;
    let mut app = match (&cli.path, cli.lang.as_deref()) {
        (Some(path), Some(lang)) => App::load_with(path, lang, &cli.variants, Some(store), config)?,
        (Some(path), None) => App::load_auto_with(path, &cli.variants, Some(store), config)?,
        (None, _) => App::shelf_with(Some(store), cli.lang.as_deref().unwrap_or("en"), config)?,
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
