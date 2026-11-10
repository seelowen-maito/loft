//! Loft's HTTP API, as the web app uses it, plus its live event stream.

use std::io::{BufRead, BufReader};
use std::time::Duration;

use anyhow::{Result, anyhow};
use serde_json::{Value, json};

#[derive(Clone)]
pub struct Loft {
    pub base: String,
    http: ureq::Agent,
}

/// What the event thread hands the app.
pub enum Live {
    Connected,
    Disconnected(String),
    Event(Value),
}

impl Loft {
    pub fn new(base: &str) -> Self {
        let http = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(5))
            .timeout(Duration::from_secs(60))
            .user_agent("loft-herdr")
            .build();
        Self { base: base.to_string(), http }
    }

    fn call(&self, method: &str, path: &str, body: Option<Value>) -> Result<Value> {
        let req = self.http.request(method, &format!("{}/api{path}", self.base));
        let res = match body {
            Some(b) => req.send_json(b),
            None if method == "GET" || method == "DELETE" => req.call(),
            None => req.send_json(json!({})),
        };
        match res {
            Ok(r) => {
                let text = r.into_string()?;
                Ok(if text.trim().is_empty() { Value::Null } else { serde_json::from_str(&text).unwrap_or(Value::String(text)) })
            }
            Err(ureq::Error::Status(code, r)) => {
                let text = r.into_string().unwrap_or_default();
                let msg = serde_json::from_str::<Value>(&text)
                    .ok()
                    .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(str::to_string))
                    .unwrap_or(text);
                Err(anyhow!("{code}: {}", if msg.is_empty() { "request failed".into() } else { msg }))
            }
            Err(e) => Err(anyhow!("can't reach Loft at {}: {e}", self.base)),
        }
    }

    pub fn get(&self, path: &str) -> Result<Value> {
        self.call("GET", path, None)
    }
    pub fn post(&self, path: &str, body: Value) -> Result<Value> {
        self.call("POST", path, Some(body))
    }

    pub fn agents(&self) -> Result<Vec<Value>> {
        Ok(self.get("/agents")?.as_array().cloned().unwrap_or_default())
    }
    pub fn agent(&self, id: &str) -> Result<Value> {
        self.get(&format!("/agents/{id}"))
    }
    pub fn messages(&self, id: &str) -> Result<Vec<Value>> {
        Ok(self.get(&format!("/agents/{id}/messages?limit=400"))?.as_array().cloned().unwrap_or_default())
    }
    pub fn send(&self, id: &str, text: &str, client_id: &str) -> Result<Value> {
        self.post(&format!("/agents/{id}/messages"), json!({ "text": text, "images": [], "client_id": client_id }))
    }
    pub fn grill_start(&self, id: &str, text: &str, client_id: &str) -> Result<Value> {
        self.post(&format!("/agents/{id}/messages"), json!({ "text": text, "client_id": client_id, "meta": { "grill": { "role": "start_request" } } }))
    }
    pub fn action(&self, id: &str, what: &str) -> Result<Value> {
        self.post(&format!("/agents/{id}/{what}"), json!({}))
    }
    pub fn compact(&self, id: &str, instructions: &str) -> Result<Value> {
        self.post(&format!("/agents/{id}/compact"), json!({ "instructions": instructions }))
    }
    pub fn resume_session(&self, id: &str, session: &str) -> Result<Value> {
        self.post(&format!("/agents/{id}/resume-session"), json!({ "session_id": session }))
    }
    pub fn clear_messages(&self, id: &str) -> Result<Value> {
        self.call("DELETE", &format!("/agents/{id}/messages"), None)
    }
    pub fn update_agent(&self, id: &str, patch: Value) -> Result<Value> {
        self.call("PATCH", &format!("/agents/{id}"), Some(patch))
    }
    pub fn permissions(&self, id: &str) -> Result<Vec<Value>> {
        Ok(self.get(&format!("/permissions?agent_id={id}"))?.as_array().cloned().unwrap_or_default())
    }
    pub fn decide(&self, request: &str, decision: &str) -> Result<Value> {
        self.post(&format!("/permissions/{request}"), json!({ "decision": decision }))
    }
    pub fn memories(&self, id: &str) -> Result<Vec<Value>> {
        Ok(self.get(&format!("/agents/{id}/memories"))?.as_array().cloned().unwrap_or_default())
    }
    pub fn add_memory(&self, id: &str, title: &str) -> Result<Value> {
        self.post(&format!("/agents/{id}/memories"), json!({ "title": title, "content": "", "tags": "" }))
    }
    pub fn delete_memory(&self, id: &str, mid: i64) -> Result<Value> {
        self.call("DELETE", &format!("/agents/{id}/memories/{mid}"), None)
    }
    pub fn files(&self, id: &str) -> Result<Vec<Value>> {
        Ok(self.get(&format!("/agents/{id}/files"))?.as_array().cloned().unwrap_or_default())
    }

    /// Reads `/api/events` forever on its own thread, reconnecting after a drop.
    pub fn stream(&self, send: impl Fn(Live) -> bool + Send + 'static) {
        let base = self.base.clone();
        std::thread::spawn(move || {
            let http = ureq::AgentBuilder::new().timeout_connect(Duration::from_secs(5)).user_agent("loft-herdr").build();
            let mut wait = 1;
            loop {
                match http.get(&format!("{base}/api/events")).set("accept", "text/event-stream").call() {
                    Ok(res) => {
                        wait = 1;
                        if !send(Live::Connected) {
                            return;
                        }
                        let mut data = String::new();
                        for line in BufReader::new(res.into_reader()).lines() {
                            let Ok(line) = line else { break };
                            if let Some(d) = line.strip_prefix("data:") {
                                data.push_str(d.trim_start());
                            } else if line.is_empty() && !data.is_empty() {
                                if let Ok(v) = serde_json::from_str::<Value>(&data)
                                    && !send(Live::Event(v))
                                {
                                    return;
                                }
                                data.clear();
                            }
                        }
                        if !send(Live::Disconnected("connection closed".into())) {
                            return;
                        }
                    }
                    Err(e) => {
                        if !send(Live::Disconnected(e.to_string())) {
                            return;
                        }
                    }
                }
                std::thread::sleep(Duration::from_secs(wait));
                wait = (wait * 2).min(15);
            }
        });
    }
}
