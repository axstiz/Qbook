//! Выравнивание варианта документа к базовому: banded Needleman–Wunsch.
//!
//! Перевод — второй прогон того же источника, поэтому блоки почти совпадают 1:1,
//! но абзацы могут разойтись: merge, split, вставки редактора. Карта хранит только
//! точные пары; несопоставленные блоки переводятся по общему прогрессу чтения,
//! зажатому в интервал ближайших сопоставленных соседей — так карта остаётся
//! монотонной.

use std::collections::HashMap;

use crate::model::{Anchor, Block, BlockKind, Document};

const GAP_OPEN: f32 = 0.9;
const GAP_EXTEND: f32 = 0.3;
/// «Недостижимая» клетка DP: настоящее `INF` ломает `min`-выражения.
const INF: f32 = 1e30;
/// Совпадение считается полным, пока лог-длины блоков ближе этого порога.
const LOG_LEN_MATCH: f32 = 0.5;

/// Признаки блока — из них складывается стоимость пары в DP.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Features {
    pub chars: usize,
    pub sentences: u32,
    pub heading: bool,
}

pub fn features(block: &Block) -> Features {
    Features {
        chars: block.char_len(),
        sentences: sentences(&block.text),
        heading: matches!(block.kind, BlockKind::Heading(_)),
    }
}

/// Конец предложения — группа терминаторов `.!?…`; точка внутри числа не считается.
fn sentences(text: &str) -> u32 {
    let chars: Vec<char> = text.chars().collect();
    let mut count = 0;
    let mut i = 0;
    while i < chars.len() {
        if !matches!(chars[i], '.' | '!' | '?' | '…') {
            i += 1;
            continue;
        }
        if chars[i] == '.'
            && i > 0
            && i + 1 < chars.len()
            && chars[i - 1].is_numeric()
            && chars[i + 1].is_numeric()
        {
            i += 1;
            continue;
        }
        while i < chars.len() && matches!(chars[i], '.' | '!' | '?' | '…') {
            i += 1;
        }
        count += 1;
    }
    count
}

/// Стоимость пары «базовый блок ↔ блок варианта»: длина (с поправкой на общий
/// масштаб перевода), число предложений и совпадение типа блока.
fn sub_cost(a: &Features, b: &Features, ratio: f32) -> f32 {
    let len = (1.0 + a.chars as f32 * ratio).ln() - (1.0 + b.chars as f32).ln();
    let len = 0.5 * len.abs() / 2f32.ln();
    let sent = 0.3 * (a.sentences.abs_diff(b.sentences) as f32 / 3.0).min(1.0);
    let kind = 0.2 * 0.6 * u8::from(a.heading != b.heading) as f32;
    len + sent + kind
}

/// Карта соответствия базового документа и варианта.
#[derive(Debug, Clone)]
pub struct Alignment {
    /// Точная пара базового блока с блоком варианта. Фолбэк карту не дополняет.
    pub base_to_var: Vec<Option<usize>>,
    /// Обратная карта: блок варианта → базовый блок (только точные пары).
    pub var_to_base: HashMap<usize, usize>,
    /// Доля базовых блоков с точной парой.
    coverage: f32,
    n_base: usize,
    n_var: usize,
    /// Ближайшая точная пара слева/справа — зажимает фолбэк в интервал соседей.
    base_prev: Vec<Option<usize>>,
    base_next: Vec<Option<usize>>,
    var_exact: Vec<Option<usize>>,
    var_prev: Vec<Option<usize>>,
    var_next: Vec<Option<usize>>,
}

impl Alignment {
    fn from_pairs(pairs: &[(usize, usize)], n_base: usize, n_var: usize) -> Self {
        let mut base_to_var = vec![None; n_base];
        let mut var_to_base = HashMap::new();
        let mut matched = 0;
        for &(i, j) in pairs {
            if i < n_base && j < n_var && base_to_var[i].is_none() {
                base_to_var[i] = Some(j);
                var_to_base.insert(j, i);
                matched += 1;
            }
        }
        let coverage = if n_base == 0 { 1.0 } else { matched as f32 / n_base as f32 };
        let (base_prev, base_next) = neighbors(&base_to_var);
        let mut var_exact = vec![None; n_var];
        for (&j, &i) in &var_to_base {
            var_exact[j] = Some(i);
        }
        let (var_prev, var_next) = neighbors(&var_exact);
        Self {
            base_to_var,
            var_to_base,
            coverage,
            n_base,
            n_var,
            base_prev,
            base_next,
            var_exact,
            var_prev,
            var_next,
        }
    }

    /// Восстановление из кэшированной карты (хранилище): пары собираются заново.
    pub fn from_map(map: &[Option<usize>], n_base: usize, n_var: usize) -> Self {
        let pairs: Vec<(usize, usize)> =
            map.iter().enumerate().filter_map(|(i, &j)| j.map(|j| (i, j))).collect();
        Self::from_pairs(&pairs, n_base, n_var)
    }

    pub fn coverage(&self) -> f32 {
        self.coverage
    }

    /// Базовый якорь → вариант: для сопоставленного блока доля сохраняется,
    /// для несопоставленного позиция берётся из общего прогресса чтения.
    pub fn translate_base_to_var(&self, anchor: Anchor) -> Anchor {
        self.translate(
            anchor,
            self.n_base,
            self.n_var,
            &self.base_to_var,
            &self.base_prev,
            &self.base_next,
        )
    }

    /// Вариант → база: обратный перевод с теми же правилами.
    pub fn translate_var_to_base(&self, anchor: Anchor) -> Anchor {
        self.translate(
            anchor,
            self.n_var,
            self.n_base,
            &self.var_exact,
            &self.var_prev,
            &self.var_next,
        )
    }

    fn translate(
        &self,
        anchor: Anchor,
        n_from: usize,
        n_to: usize,
        exact: &[Option<usize>],
        prev: &[Option<usize>],
        next: &[Option<usize>],
    ) -> Anchor {
        if n_from == 0 || n_to == 0 {
            return Anchor::START;
        }
        let block = anchor.block.min(n_from - 1);
        if let Some(mapped) = exact[block] {
            return Anchor::new(mapped, anchor.frac);
        }
        let progress = (block as f32 + anchor.frac) / n_from as f32;
        let mut pos = progress * n_to as f32;
        let lo = prev[block].map_or(0, |v| v + 1);
        let hi = next[block].map_or(n_to - 1, |v| v.saturating_sub(1));
        if lo <= hi {
            pos = pos.clamp(lo as f32, (hi + 1) as f32);
        }
        let mut target = pos.floor() as usize;
        if target >= n_to {
            target = n_to - 1;
        }
        if lo <= hi {
            target = target.clamp(lo, hi);
        }
        let frac = (pos - target as f32).clamp(0.0, 1.0);
        Anchor::new(target, frac)
    }
}

/// Выравнивание: сначала быстрый путь (почти совпадающие документы), затем DP.
pub fn align(base: &Document, var: &Document) -> Alignment {
    if let Some(pairs) = fast_path(base, var) {
        return Alignment::from_pairs(&pairs, base.len(), var.len());
    }
    align_dp(base, var)
}

/// Только DP, без быстрого пути — используется в тестах и когда быстрый путь отклонён.
pub fn align_dp(base: &Document, var: &Document) -> Alignment {
    let n = base.len();
    let m = var.len();
    if n == 0 || m == 0 {
        return Alignment::from_pairs(&[], n, m);
    }
    let base_features: Vec<Features> = base.blocks().iter().map(features).collect();
    let var_features: Vec<Features> = var.blocks().iter().map(features).collect();
    let total_base: usize = base_features.iter().map(|f| f.chars).sum();
    let ratio = if total_base == 0 {
        1.0
    } else {
        var_features.iter().map(|f| f.chars).sum::<usize>() as f32 / total_base as f32
    };

    let radius = (n / 8).max(64);
    let mut rows: Vec<Row> = Vec::with_capacity(n + 1);

    let (lo, hi) = band_row(0, n, m, radius);
    let mut first = Row::new(lo, hi);
    for j in lo..=hi {
        let k = first.idx(j).expect("клетка строки в полосе");
        if j == 0 {
            first.vals[k] = [0.0, INF, INF];
        } else {
            first.vals[k][2] = GAP_OPEN + (j - 1) as f32 * GAP_EXTEND;
            first.back[k][2] = if j == 1 { 0 } else { 2 };
        }
    }
    rows.push(first);

    for i in 1..=n {
        let (lo, hi) = band_row(i, n, m, radius);
        let mut row = Row::new(lo, hi);
        let prev = &rows[i - 1];
        for j in lo..=hi {
            let k = row.idx(j).expect("клетка строки в полосе");
            if j > 0
                && let Some(pk) = prev.idx(j - 1)
            {
                let (best, state) = min3(prev.vals[pk]);
                if best < INF {
                    row.vals[k][0] =
                        best + sub_cost(&base_features[i - 1], &var_features[j - 1], ratio);
                    row.back[k][0] = state;
                }
            }
            if let Some(pk) = prev.idx(j) {
                let pv = prev.vals[pk];
                let (best, state) = min3([pv[0] + GAP_OPEN, pv[1] + GAP_EXTEND, pv[2] + GAP_OPEN]);
                if best < INF {
                    row.vals[k][1] = best;
                    row.back[k][1] = state;
                }
            }
            if j > lo {
                let lv = row.vals[k - 1];
                let (best, state) = min3([lv[0] + GAP_OPEN, lv[1] + GAP_OPEN, lv[2] + GAP_EXTEND]);
                if best < INF {
                    row.vals[k][2] = best;
                    row.back[k][2] = state;
                }
            }
        }
        rows.push(row);
    }

    let last = &rows[n];
    let k = last.idx(m).expect("конец полосы внутри строки");
    let (_, mut state) = min3(last.vals[k]);
    if last.vals[k][state as usize] >= INF {
        return Alignment::from_pairs(&[], n, m);
    }
    let mut pairs = Vec::new();
    let (mut i, mut j) = (n, m);
    while i > 0 || j > 0 {
        let row = &rows[i];
        let k = row.idx(j).expect("клетка трекбэка в полосе");
        match state {
            0 => {
                pairs.push((i - 1, j - 1));
                state = row.back[k][0];
                i -= 1;
                j -= 1;
            }
            1 => {
                state = row.back[k][1];
                i -= 1;
            }
            2 => {
                state = row.back[k][2];
                j -= 1;
            }
            _ => unreachable!("трекбэк дошёл до старта не на (0,0)"),
        }
    }
    pairs.reverse();
    Alignment::from_pairs(&pairs, n, m)
}

/// Быстрый путь: те же блоки и близкие длины на ≥ 90 % позиций — карта тождественна.
fn fast_path(base: &Document, var: &Document) -> Option<Vec<(usize, usize)>> {
    if base.len() != var.len() {
        return None;
    }
    let mut matches = 0;
    for (b, v) in base.blocks().iter().zip(var.blocks()) {
        let len_delta = (1.0 + b.char_len() as f32).ln() - (1.0 + v.char_len() as f32).ln();
        if b.kind == v.kind && len_delta.abs() <= LOG_LEN_MATCH {
            matches += 1;
        }
    }
    let n = base.len();
    if matches * 10 >= n * 9 { Some((0..n).map(|i| (i, i)).collect()) } else { None }
}

/// Одна строка полосы: значения трёх состояний и предки для трекбэка.
struct Row {
    lo: usize,
    hi: usize,
    /// Состояния на ячейке: 0 — M (диагональ), 1 — X (гэп варианта), 2 — Y (гэп базы).
    vals: Vec<[f32; 3]>,
    /// Предыдущее состояние (0..2) либо 3 — старт в (0,0).
    back: Vec<[u8; 3]>,
}

impl Row {
    fn new(lo: usize, hi: usize) -> Self {
        let cells = hi - lo + 1;
        Self { lo, hi, vals: vec![[INF; 3]; cells], back: vec![[3; 3]; cells] }
    }

    fn idx(&self, j: usize) -> Option<usize> {
        (self.lo <= j && j <= self.hi).then_some(j - self.lo)
    }
}

/// Диапазон столбцов строки `i` полосы вокруг диагонали `|j − i·m/n| ≤ radius`.
fn band_row(i: usize, n: usize, m: usize, radius: usize) -> (usize, usize) {
    let center = (i as f64 * m as f64 / n as f64).round() as usize;
    (center.saturating_sub(radius).min(m), (center + radius).min(m))
}

/// Минимум из трёх состояний с точным tie-break в пользу M (детерминирует трекбэк).
fn min3(v: [f32; 3]) -> (f32, u8) {
    let mut best = v[0];
    let mut state = 0;
    if v[1] < best {
        best = v[1];
        state = 1;
    }
    if v[2] < best {
        best = v[2];
        state = 2;
    }
    (best, state)
}

/// Для каждого блока — ближайшая точная пара слева и справа.
fn neighbors(exact: &[Option<usize>]) -> (Vec<Option<usize>>, Vec<Option<usize>>) {
    let mut prev = vec![None; exact.len()];
    let mut last = None;
    for (i, slot) in exact.iter().enumerate() {
        prev[i] = last;
        if slot.is_some() {
            last = *slot;
        }
    }
    let mut next = vec![None; exact.len()];
    let mut last = None;
    for i in (0..exact.len()).rev() {
        next[i] = last;
        if exact[i].is_some() {
            last = exact[i];
        }
    }
    (prev, next)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::BlockKind;

    #[test]
    fn sentences_count_terminator_groups() {
        assert_eq!(sentences("Один. Два! Три?"), 3);
        assert_eq!(sentences("многоточие... конец"), 1);
        assert_eq!(sentences("Привет…"), 1);
        assert_eq!(sentences("3.14 — число"), 0);
        assert_eq!(sentences("3.14. Это число."), 2);
        assert_eq!(sentences(""), 0);
    }

    #[test]
    fn features_describe_the_block() {
        let heading = features(&Block::new(BlockKind::Heading(2), "Заголовок"));
        assert!(heading.heading);
        assert_eq!(heading.chars, 9);
        assert_eq!(heading.sentences, 0);

        let para = features(&Block::new(BlockKind::Paragraph, "Абзац один. Абзац два."));
        assert!(!para.heading);
        assert_eq!(para.sentences, 2);
        assert_eq!(para.chars, 22);
    }
}
