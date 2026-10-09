use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use qbook::app::ReaderFocus;
use qbook::keys::{Group, KeyCmd, corner_hints, current_bindings, match_key};

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn shift(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::SHIFT)
}

fn cmds(focus: ReaderFocus, search: bool) -> Vec<KeyCmd> {
    current_bindings(focus, search).into_iter().map(|b| b.cmd).collect()
}

#[test]
fn rename_only_lives_in_the_bookmarks_pane() {
    let text = cmds(ReaderFocus::Text, false);
    let bookmarks = cmds(ReaderFocus::Bookmarks, false);
    assert!(!text.contains(&KeyCmd::RenameBookmark), "в тексте r не переименовывает");
    assert!(bookmarks.contains(&KeyCmd::RenameBookmark), "в заметках r переименовывает");
}

#[test]
fn bookmarks_recolor_key_is_labeled_in_the_bar() {
    let bindings = current_bindings(ReaderFocus::Bookmarks, false);
    let recolor = bindings.iter().find(|b| b.cmd == KeyCmd::RecolorNext).expect("c есть");
    let bar = recolor.bar.expect("c в баре");
    assert_eq!(bar.label, "c/C");
    assert_eq!(bar.desc, Some("цвет"), "описание цвета у c/C");
}

#[test]
fn search_navigation_wins_over_bookmark_jump() {
    let idle = match_key(&current_bindings(ReaderFocus::Text, false), key(KeyCode::Char('n')));
    assert_eq!(idle, Some(KeyCmd::JumpBookmarkNext));

    let busy = match_key(&current_bindings(ReaderFocus::Text, true), key(KeyCode::Char('n')));
    assert_eq!(busy, Some(KeyCmd::SearchNext), "при поиске n листает результаты");
    let back = match_key(&current_bindings(ReaderFocus::Text, true), key(KeyCode::Char('N')));
    assert_eq!(back, Some(KeyCmd::SearchPrev));
}

#[test]
fn shift_digit_panel_toggles_are_blocked_while_searching() {
    let busy = current_bindings(ReaderFocus::Text, true);
    for k in [shift('3'), key(KeyCode::Char('#')), key(KeyCode::Char('№'))] {
        assert_ne!(match_key(&busy, k), Some(KeyCmd::ToggleBookmarks), "поиск держит заметки");
    }
    for k in [shift('4'), key(KeyCode::Char('$')), key(KeyCode::Char(';'))] {
        assert_ne!(match_key(&busy, k), Some(KeyCmd::ToggleCommands), "поиск держит команды");
    }
    let idle = current_bindings(ReaderFocus::Text, false);
    assert_eq!(match_key(&idle, shift('3')), Some(KeyCmd::ToggleBookmarks));
}

#[test]
fn quit_and_shelf_need_capital_letters_in_reader() {
    let bindings = current_bindings(ReaderFocus::Text, false);
    assert_eq!(match_key(&bindings, key(KeyCode::Char('Q'))), Some(KeyCmd::Quit));
    assert_eq!(match_key(&bindings, key(KeyCode::Char('S'))), Some(KeyCmd::GoShelf));
    assert_eq!(match_key(&bindings, key(KeyCode::Char('q'))), None, "строчная q не выходит");
    assert_eq!(
        match_key(&bindings, key(KeyCode::Char('s'))),
        None,
        "строчная s не открывает полку"
    );
}

#[test]
fn quit_and_shelf_are_corner_hints_not_bar() {
    let bindings = current_bindings(ReaderFocus::Text, false);
    for cmd in [KeyCmd::Quit, KeyCmd::GoShelf] {
        let binding = bindings.iter().find(|b| b.cmd == cmd).expect("привязка есть");
        assert!(binding.bar.is_none(), "Q/S не в нижнем баре: {cmd:?}");
        assert!(binding.corner.is_some(), "Q/S в углу: {cmd:?}");
    }
    assert_eq!(
        corner_hints(ReaderFocus::Text, false),
        vec![("S", "полка"), ("Q", "выход")],
        "порядок угла: S, затем Q"
    );
}

#[test]
fn search_keeps_note_and_select_hints_but_hides_language() {
    let busy = current_bindings(ReaderFocus::Text, true);
    let labeled =
        |cmd: KeyCmd| busy.iter().find(|b| b.cmd == cmd).and_then(|b| b.bar).map(|bar| bar.label);
    assert_eq!(labeled(KeyCmd::StartNotePick), Some("b"), "b остаётся в баре при поиске");
    assert_eq!(labeled(KeyCmd::StartSelect), Some("v"), "v остаётся в баре при поиске");
    assert_eq!(labeled(KeyCmd::NextLang), None, "t прячется из бара при поиске");
}

#[test]
fn bar_labels_are_unique_except_the_grouped_jk() {
    for focus in [
        ReaderFocus::Text,
        ReaderFocus::Toc,
        ReaderFocus::Bookmarks,
        ReaderFocus::Commands,
        ReaderFocus::SearchResults,
    ] {
        for search in [false, true] {
            let mut labels: Vec<&str> = current_bindings(focus, search)
                .iter()
                .filter(|b| b.group != Group::Digits)
                .filter_map(|b| b.bar.map(|bar| bar.label))
                .collect();
            let jk = labels.iter().filter(|l| **l == "j/k").count();
            labels.retain(|l| *l != "j/k");
            labels.sort_unstable();
            let mut unique = labels.clone();
            unique.dedup();
            assert_eq!(labels, unique, "дубли меток бара: {focus:?}/{search}");
            assert!(jk <= 2, "j/k не больше двух привязок: {jk}");
        }
    }
}
