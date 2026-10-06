//! Выравнивание: синтетические сценарии (метрика M1) и перевод якорей.

use qbook::align::{align, align_dp};
use qbook::model::{Anchor, Block, BlockKind, Document};

fn doc(texts: &[&str]) -> Document {
    Document::new("ru", "T", texts.iter().map(|t| Block::new(BlockKind::Paragraph, *t)).collect())
}

fn six_paragraphs() -> Vec<&'static str> {
    vec![
        "Начало первой главы повести о странствиях героя.",
        "Второй абзац рассказывает о событиях того дня.",
        "Третий имеет совсем другой размер текста для разнообразия.",
        "Четвёртый снова короткий.",
        "Пятый абзац — длинный, он состоит из многих слов и предложений.",
        "Шестой закрывает главу перед следующей.",
    ]
}

#[test]
fn dp_gives_identity_on_exact_match() {
    let texts = six_paragraphs();
    let base = doc(&texts);
    let var = doc(&texts);
    let a = align_dp(&base, &var);
    assert_eq!(a.base_to_var, (0..6).map(Some).collect::<Vec<_>>());
    assert_eq!(a.var_to_base.len(), 6);
    assert_eq!(a.var_to_base[&4], 4);
    assert!((a.coverage() - 1.0).abs() < 1e-6);
}

#[test]
fn dp_shifts_by_three() {
    let base = doc(&six_paragraphs());
    let var = doc(&[
        "Вступительное вступление первое.",
        "Вступительное вступление второе.",
        "Вступительное вступление третье.",
        "Начало первой главы повести о странствиях героя.",
        "Второй абзац рассказывает о событиях того дня.",
        "Третий имеет совсем другой размер текста для разнообразия.",
        "Четвёртый снова короткий.",
        "Пятый абзац — длинный, он состоит из многих слов и предложений.",
        "Шестой закрывает главу перед следующей.",
    ]);
    let a = align_dp(&base, &var);
    assert_eq!(a.base_to_var, (3..9).map(Some).collect::<Vec<_>>());
    assert!((a.coverage() - 1.0).abs() < 1e-6);
    assert_eq!(a.translate_base_to_var(Anchor::START), Anchor::new(3, 0.0));
    assert_eq!(a.translate_var_to_base(Anchor::new(4, 0.25)), Anchor::new(1, 0.25));
}

#[test]
fn dp_merge_leaves_one_block_unmatched() {
    let base = doc(&[
        "Абзац первый довольно длинный и подробный.",
        "Абзац второй тоже длинный, но совсем иной.",
    ]);
    let var = doc(&[
        "Абзац первый довольно длинный и подробный. Абзац второй тоже длинный, но совсем иной.",
    ]);
    let a = align_dp(&base, &var);
    let matched: Vec<usize> =
        a.base_to_var.iter().enumerate().filter_map(|(i, x)| x.map(|_| i)).collect();
    assert_eq!(matched.len(), 1, "merge: ровно одна пара, {:?}", a.base_to_var);
    assert!((a.coverage() - 0.5).abs() < 1e-6);
}

#[test]
fn dp_split_maps_a_single_block_forward() {
    let base = doc(&["Первая половина текста. Вторая половина тоже здесь."]);
    let var = doc(&["Первая половина текста.", "Вторая половина тоже здесь."]);
    let a = align_dp(&base, &var);
    assert_eq!(a.base_to_var, vec![Some(1)]);
    assert!((a.coverage() - 1.0).abs() < 1e-6);
    assert_eq!(a.var_to_base.get(&0), None, "первая половина несопоставлена");
    assert_eq!(a.var_to_base.get(&1), Some(&0));
}

#[test]
fn dp_tolerates_one_point_two_translation() {
    let base = doc(&six_paragraphs());
    let longer: Vec<String> = six_paragraphs()
        .iter()
        .map(|t| format!("{t} Дополнительно к тексту добавлены слова."))
        .collect();
    let refs: Vec<&str> = longer.iter().map(String::as_str).collect();
    let var = doc(&refs);
    let a = align_dp(&base, &var);
    assert_eq!(a.base_to_var, (0..6).map(Some).collect::<Vec<_>>(), "×1.2 длины даёт ту же карту");
    assert!((a.coverage() - 1.0).abs() < 1e-6);
}

fn assert_same_map(base: &Document, var: &Document) {
    let fast = align(base, var);
    let dp = align_dp(base, var);
    assert_eq!(fast.base_to_var, dp.base_to_var, "быстрый путь и DP дают одну карту");
    assert_eq!(fast.var_to_base, dp.var_to_base);
    assert!((fast.coverage() - dp.coverage()).abs() < 1e-6);
}

#[test]
fn fast_path_matches_dp() {
    // Идентичные документы.
    let texts = six_paragraphs();
    assert_same_map(&doc(&texts), &doc(&texts));

    // Ровно 90% совпадений: один блок испорчен, лог-длина вышла за порог.
    let mut broken = six_paragraphs();
    broken[2] = "Совсем иной текст, растянутый до другой длины, чем было в оригинале, он заметно длиннее и совсем иначе написан.";
    assert_same_map(&doc(&six_paragraphs()), &doc(&broken));

    // Разное число блоков: быстрый путь обязан отказаться и совпасть с DP.
    assert_same_map(&doc(&six_paragraphs()), &doc(&broken[..5]));
}

#[test]
fn aligned_anchor_keeps_frac_round_trip() {
    let texts = six_paragraphs();
    let a = align(&doc(&texts), &doc(&texts));
    for (block, frac) in [(0, 0.0), (3, 0.25), (5, 1.0)] {
        let anchor = Anchor::new(block, frac);
        let forward = a.translate_base_to_var(anchor);
        assert_eq!(forward, anchor);
        assert_eq!(a.translate_var_to_base(forward), anchor, "round-trip точен для пар");
    }
}

#[test]
fn unmapped_anchor_follows_global_progress() {
    let base = doc(&[
        "Абзац первый довольно длинный и подробный.",
        "Абзац второй тоже длинный, но совсем иной.",
    ]);
    let var = doc(&[
        "Абзац первый довольно длинный и подробный. Абзац второй тоже длинный, но совсем иной.",
    ]);
    let a = align_dp(&base, &var);
    for block in 0..2 {
        if a.base_to_var[block].is_some() {
            continue;
        }
        let t = a.translate_base_to_var(Anchor::new(block, 0.5));
        assert!(t.block < var.len(), "индекс в границах варианта");
        let before = (block as f32 + 0.5) / base.len() as f32;
        let after = (t.block as f32 + t.frac) / var.len() as f32;
        assert!((before - after).abs() < 1e-5, "общий прогресс сохранён: {before} vs {after}");
    }
}

#[test]
fn empty_documents_clamp_to_start() {
    let texts = six_paragraphs();
    let base = doc(&texts);
    let empty = doc(&[]);

    let to_empty = align(&base, &empty);
    assert_eq!(to_empty.translate_base_to_var(Anchor::new(2, 0.5)), Anchor::START);
    assert_eq!(to_empty.translate_var_to_base(Anchor::START), Anchor::START);

    let both = align(&empty, &empty);
    assert!((both.coverage() - 1.0).abs() < 1e-6);
    assert_eq!(both.translate_base_to_var(Anchor::new(5, 0.5)), Anchor::START);
}

#[test]
fn out_of_range_anchor_is_clamped() {
    let texts = six_paragraphs();
    let a = align(&doc(&texts), &doc(&texts));
    assert_eq!(a.translate_base_to_var(Anchor::new(99, 0.9)), Anchor::new(5, 0.9));
    assert_eq!(a.translate_var_to_base(Anchor::new(99, 0.9)), Anchor::new(5, 0.9));
}
