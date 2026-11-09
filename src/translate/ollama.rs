//! Движок перевода через OpenAI-совместимый бэкенд (Ollama, llama.cpp, LM Studio).
//!
//! Партия текстов уходит одним chat-запросом: модели дают нумерованный список и
//! просят вернуть те же номера. Если модель сорвалась с формата, партия
//! переигрывается по одному тексту.

use serde_json::{Value, json};

use super::libretranslate::{Transport, UreqTransport};
use super::{Engine, TranslateError};

/// Текстов в одном chat-запросе по умолчанию.
pub const DEFAULT_BATCH: usize = 8;

/// Клиент OpenAI-совместимого бэкенда.
pub struct Ollama {
    base_url: String,
    model: String,
    batch_size: usize,
    transport: Box<dyn Transport>,
}

impl Ollama {
    /// Клиент по адресу сервера (например, `http://localhost:11434`) и модели.
    pub fn new(base_url: impl Into<String>, model: impl Into<String>) -> Self {
        Self::with_transport(base_url, model, Box::new(UreqTransport))
    }

    /// Клиент с подставным транспортом.
    pub fn with_transport(
        base_url: impl Into<String>,
        model: impl Into<String>,
        transport: Box<dyn Transport>,
    ) -> Self {
        Self {
            base_url: base_url.into(),
            model: model.into(),
            batch_size: DEFAULT_BATCH,
            transport,
        }
    }

    /// Текстов в одном chat-запросе.
    pub fn batch_size(mut self, size: usize) -> Self {
        self.batch_size = size.max(1);
        self
    }

    fn endpoint(&self) -> String {
        format!("{}/v1/chat/completions", self.base_url.trim_end_matches('/'))
    }

    /// Запрашивает чат, достаёт текст ответа.
    fn chat(&self, system: &str, user: &str) -> Result<String, TranslateError> {
        let url = self.endpoint();
        let body = json!({
            "model": self.model,
            "stream": false,
            "messages": [
                { "role": "system", "content": system },
                { "role": "user", "content": user },
            ],
        });
        let response = self.transport.post_json(&url, &body)?;
        response
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|list| list.first())
            .and_then(|first| first.get("message"))
            .and_then(|message| message.get("content"))
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| {
                TranslateError::Engine("ответ без choices[0].message.content".to_owned())
            })
    }
}

/// Системная инструкция: профессиональный перевод нумерованных текстов.
fn system_prompt(from: &str, to: &str) -> String {
    format!(
        "You are a professional literary translator. Translate each numbered text \
         from {from} to {to}, preserving meaning, line breaks, punctuation and numbering. \
         Placeholders like [[0]] must be kept verbatim. Return ONLY the numbered \
         translations, same numbers, one per line."
    )
}

/// Пользовательское сообщение: нумерованный список текстов. Переносы строк
/// внутри текста заменяются маркером ` ⏎ ` — модель не ломает нумерацию, а мы
/// восстанавливаем их в ответах.
fn numbered_prompt(texts: &[String]) -> String {
    texts
        .iter()
        .enumerate()
        .map(|(index, text)| format!("{}. {}", index + 1, text.replace('\n', " ⏎ ")))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Разбирает пронумерованный ответ (`"1. ..."` / `"1) ..."`). Число строк должно
/// совпасть с ожиданием — иначе `None`, партия переигрывается по одному.
fn parse_numbered(answer: &str, expected: usize) -> Option<Vec<String>> {
    let mut slots: Vec<Option<String>> = vec![None; expected];
    for line in answer.lines() {
        let line = line.trim();
        let Some((index, tail)) = split_number(line) else { continue };
        if let Some(slot) = slots.get_mut(index)
            && slot.is_none()
        {
            *slot = Some(tail.to_owned());
        }
    }
    let out: Vec<String> = slots.into_iter().flatten().collect();
    (out.len() == expected).then_some(out)
}

/// Отделяет номер (`12)` или `12.`) от текста строки; нумерация с единицы.
fn split_number(line: &str) -> Option<(usize, &str)> {
    let digits: String = line.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return None;
    }
    let rest = &line[digits.len()..];
    let rest = rest.strip_prefix(')').or_else(|| rest.strip_prefix('.'))?;
    let index: usize = digits.parse().ok()?;
    index.checked_sub(1).map(|index| (index, rest.trim_start()))
}

/// Восстановление переносов строк из маркеров ` ⏎ `.
fn restore_newlines(text: String) -> String {
    text.replace(" ⏎ ", "\n")
}

impl Engine for Ollama {
    fn translate(
        &self,
        texts: &[String],
        from: &str,
        to: &str,
    ) -> Result<Vec<String>, TranslateError> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let system = system_prompt(from, to);
        let mut out = Vec::with_capacity(texts.len());
        for chunk in texts.chunks(self.batch_size) {
            let answer = self.chat(&system, &numbered_prompt(chunk))?;
            if let Some(batch) = parse_numbered(&answer, chunk.len()) {
                out.extend(batch.into_iter().map(restore_newlines));
                continue;
            }
            // Модель сорвалась с формата: доигрываем партию по одному тексту.
            for text in chunk {
                let answer = self.chat(
                    &format!("{system} Return only the translation."),
                    &format!("1. {}", text.replace('\n', " ⏎ ")),
                )?;
                let one = match split_number(answer.trim()) {
                    Some((0, rest)) => rest.to_owned(),
                    _ => answer.trim().to_owned(),
                };
                out.push(restore_newlines(one));
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use serde_json::Value;

    /// Фейковый транспорт: одна функция формирует ответ на любой запрос.
    struct FakeScript {
        resolve: Box<dyn Fn(&Value) -> Value + Send + Sync>,
        calls: SharedCalls,
    }

    impl Transport for FakeScript {
        fn post_json(&self, _url: &str, body: &Value) -> Result<Value, TranslateError> {
            self.calls.lock().unwrap().push(body.clone());
            Ok((self.resolve)(body))
        }
    }

    type SharedCalls = Arc<Mutex<Vec<Value>>>;

    fn engine_with(
        resolve: impl Fn(&Value) -> Value + Send + Sync + 'static,
    ) -> (Ollama, SharedCalls) {
        let calls: SharedCalls = Arc::new(Mutex::new(Vec::new()));
        let fake = FakeScript { resolve: Box::new(resolve), calls: Arc::clone(&calls) };
        let engine = Ollama::with_transport("http://localhost:11434", "llama3", Box::new(fake));
        (engine, calls)
    }

    fn user_content(call: &Value) -> String {
        call["messages"][1]["content"].as_str().unwrap_or_default().to_owned()
    }

    /// Ответ «переводчик верхнего регистра»: номера сохраняются, текст — Caps.
    fn numbered_answer(body: &Value) -> Value {
        let content = user_content(body);
        let lines: Vec<String> = content
            .lines()
            .map(|line| match line.split_once(". ") {
                Some((number, text)) => format!("{number}. {}", text.to_uppercase()),
                None => line.to_owned(),
            })
            .collect();
        json!({ "choices": [ { "message": { "content": lines.join("\n") } } ] })
    }

    #[test]
    fn translates_batch_in_one_request_and_keeps_order() {
        let (engine, calls) = engine_with(numbered_answer);
        let input = vec!["Hello world".to_owned(), "Goodbye\nworld".to_owned()];
        let out = engine.translate(&input, "en", "ru").expect("перевод");
        assert_eq!(out[0], "HELLO WORLD");
        assert_eq!(out[1], "GOODBYE\nWORLD");
        assert_eq!(calls.lock().unwrap().len(), 1, "партия — один запрос");
    }

    #[test]
    fn splits_into_batches() {
        let (engine, calls) = engine_with(numbered_answer);
        let engine = engine.batch_size(2);
        let input: Vec<String> = ["a", "b", "c"].iter().map(|t| t.to_string()).collect();
        engine.translate(&input, "en", "ru").expect("перевод");
        let snapshot = calls.lock().unwrap();
        assert_eq!(snapshot.len(), 2, "партии: {snapshot:?}");
        assert_eq!(user_content(&snapshot[1]), "1. c");
    }

    #[test]
    fn broken_format_falls_back_to_single_requests() {
        let (engine, calls) = engine_with(|body| {
            if user_content(body).contains('\n') {
                // Многострочный запрос — мусор вместо нумерованного списка.
                json!({ "choices": [ { "message": { "content": "какая-то болтовня" } } ] })
            } else {
                numbered_answer(body)
            }
        });
        let input = vec!["alpha".to_owned(), "beta".to_owned()];
        let out = engine.translate(&input, "en", "ru").expect("перевод");
        assert_eq!(out, vec!["ALPHA", "BETA"]);
        assert_eq!(calls.lock().unwrap().len(), 3, "партия + два одиночных");
    }

    #[test]
    fn request_carries_model_stream_and_messages() {
        let (engine, calls) = engine_with(numbered_answer);
        engine.translate(&["привет".to_owned()], "en", "ru").expect("перевод");
        let body = calls.lock().unwrap()[0].clone();
        assert_eq!(body["model"], "llama3");
        assert_eq!(body["stream"], false);
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(body["messages"][1]["role"], "user");
        assert!(body["messages"][0]["content"].as_str().unwrap().contains("en to ru"));
    }

    #[test]
    fn empty_input_makes_no_request() {
        let (engine, calls) = engine_with(numbered_answer);
        assert!(engine.translate(&[], "en", "ru").unwrap().is_empty());
        assert!(calls.lock().unwrap().is_empty());
    }

    #[test]
    fn parse_numbers_both_dots_and_brackets() {
        assert_eq!(
            parse_numbered("1. раз\n2) два", 2),
            Some(vec!["раз".to_owned(), "два".to_owned()])
        );
        assert_eq!(parse_numbered("мусор", 1), None);
        assert_eq!(parse_numbered("1. раз", 2), None);
        assert_eq!(parse_numbered("0. ноль\n2. два", 2), None);
    }

    #[test]
    fn numbered_prompt_marks_newlines() {
        let prompt = numbered_prompt(&["первая\nстрока".to_owned()]);
        assert_eq!(prompt, "1. первая ⏎ строка");
    }
}
