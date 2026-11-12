use std::io::{Write, stdout};

use anyhow::Result;
use clap::Parser;
use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use crossterm::execute;

use qbook::app::App;
use qbook::cli::{Cli, Command};
use qbook::event;
use qbook::store::Store;
use qbook::ui;

fn main() -> Result<()> {
    let cli = Cli::parse();

    if let Some(Command::Translate(args)) = &cli.command {
        #[cfg(feature = "translate")]
        {
            qbook::translate::run(args)?;
            return Ok(());
        }
        #[cfg(not(feature = "translate"))]
        {
            let _ = args;
            anyhow::bail!(
                "qbook собран без поддержки перевода: пересоберите с `--features translate`"
            );
        }
    }

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
    // Картинки kitty удаляются из терминала на выходе.
    let exit_cleanup = app.exit_graphics();
    execute!(stdout(), DisableMouseCapture)?;
    ratatui::restore();
    write!(stdout(), "{exit_cleanup}")?;
    stdout().flush()?;

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
        // Передача/удаление kitty-картинок: после кадра, который нарисовал
        // placeholder-ячейки.
        let graphics = app.take_graphics();
        if !graphics.is_empty() {
            write!(stdout(), "{graphics}")?;
            stdout().flush()?;
        }
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
