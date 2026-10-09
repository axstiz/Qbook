//! Движок перевода через локальный сервер LibreTranslate (обёртка Argos).

use serde_json::{Value, json};

use super::{Engine, TranslateError};

/// Транспорт HTTP: вынесен отдельно ради тестов на фейке.
pub trait Transport: Send + Sync {
    /// Отправляет JSON и возвращает разобранный JSON-ответ.
    fn post_json(&self, url: &str, body: &Value) -> Result<Value, TranslateError>;
}

/// Транспорт поверх `ureq` (только HTTP, без TLS — сервер локальный).
pub struct UreqTransport;

impl Transport for UreqTransport {
    fn post_json(&self, url: &str, body: &Value) -> Result<Value, TranslateError> {
        let response = ureq::post(url)
            .set("Content-Type", "application/json")
            .send_string(&body.to_string())
            .map_err(|error| TranslateError::Engine(format!("запрос к {url}: {error}")))?;
        let text = response
            .into_string()
            .map_err(|error| TranslateError::Engine(format!("ответ {url}: {error}")))?;
        serde_json::from_str(&text)
            .map_err(|error| TranslateError::Engine(format!("разбор ответа {url}: {error}")))
    }
}

/// Клиент LibreTranslate.
pub struct LibreTranslate {
    base_url: String,
    batch_size: usize,
    transport: Box<dyn Transport>,
}

impl LibreTranslate {
    /// Клиент по адресу сервера (например, `http://localhost:5000`).
    pub fn new(base_url: impl Into<String>) -> Self {
        Self::with_transport(base_url, Box::new(UreqTransport))
    }

    /// Клиент с подставным транспортом.
    pub fn with_transport(base_url: impl Into<String>, transport: Box<dyn Transport>) -> Self {
        Self { base_url: base_url.into(), batch_size: 32, transport }
    }

    /// Размер партии текстов в одном запросе.
    pub fn batch_size(mut self, size: usize) -> Self {
        self.batch_size = size.max(1);
        self
    }

    fn endpoint(&self) -> String {
        format!("{}/translate", self.base_url.trim_end_matches('/'))
    }
}

impl Engine for LibreTranslate {
    fn translate(
        &self,
        texts: &[String],
        from: &str,
        to: &str,
    ) -> Result<Vec<String>, TranslateError> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let url = self.endpoint();
        let mut out = Vec::with_capacity(texts.len());
        for chunk in texts.chunks(self.batch_size) {
            let query =
                Value::Array(chunk.iter().map(|text| Value::String(text.clone())).collect());
            let body = json!({ "q": query, "source": from, "target": to, "format": "text" });
            let response = self.transport.post_json(&url, &body)?;
            let list =
                response.get("translatedText").and_then(Value::as_array).ok_or_else(|| {
                    TranslateError::Engine("ответ без поля translatedText".to_owned())
                })?;
            if list.len() != chunk.len() {
                return Err(TranslateError::Engine(format!(
                    "ожидалось {} переводов, получено {}",
                    chunk.len(),
                    list.len()
                )));
            }
            for item in list {
                let text = item
                    .as_str()
                    .ok_or_else(|| TranslateError::Engine("translatedText не строка".to_owned()))?;
                out.push(text.to_owned());
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    #[derive(Default)]
    struct Shared {
        calls: Mutex<Vec<Value>>,
        malformed: bool,
    }

    struct Fake(Arc<Shared>);

    impl Transport for Fake {
        fn post_json(&self, _url: &str, body: &Value) -> Result<Value, TranslateError> {
            self.0.calls.lock().unwrap().push(body.clone());
            if self.0.malformed {
                return Ok(json!({}));
            }
            let texts = body["q"].as_array().unwrap();
            let translated: Vec<Value> = texts
                .iter()
                .map(|text| Value::String(text.as_str().unwrap().to_uppercase()))
                .collect();
            Ok(json!({ "translatedText": translated }))
        }
    }

    fn engine(batch: usize) -> (LibreTranslate, Arc<Shared>) {
        let shared = Arc::new(Shared::default());
        let engine = LibreTranslate::with_transport(
            "http://localhost:5000/",
            Box::new(Fake(Arc::clone(&shared))),
        )
        .batch_size(batch);
        (engine, shared)
    }

    #[test]
    fn translates_and_keeps_order() {
        let (engine, _) = engine(32);
        let input = vec!["one".to_owned(), "two".to_owned()];
        let out = engine.translate(&input, "en", "ru").unwrap();
        assert_eq!(out, vec!["ONE", "TWO"]);
    }

    #[test]
    fn splits_into_batches() {
        let (engine, shared) = engine(2);
        let input = vec!["a".to_owned(), "b".to_owned(), "c".to_owned()];
        let out = engine.translate(&input, "en", "ru").unwrap();
        assert_eq!(out, vec!["A", "B", "C"]);
        assert_eq!(shared.calls.lock().unwrap().len(), 2);
    }

    #[test]
    fn empty_input_makes_no_request() {
        let (engine, shared) = engine(32);
        assert!(engine.translate(&[], "en", "ru").unwrap().is_empty());
        assert!(shared.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn request_carries_languages_and_format() {
        let (engine, shared) = engine(32);
        engine.translate(&["hi".to_owned()], "de", "ru").unwrap();
        let calls = shared.calls.lock().unwrap();
        let body = &calls[0];
        assert_eq!(body["source"], "de");
        assert_eq!(body["target"], "ru");
        assert_eq!(body["format"], "text");
    }

    #[test]
    fn missing_field_is_error() {
        let shared = Arc::new(Shared { malformed: true, ..Shared::default() });
        let engine =
            LibreTranslate::with_transport("http://localhost:5000", Box::new(Fake(shared)));
        assert!(engine.translate(&["x".to_owned()], "en", "ru").is_err());
    }
}
