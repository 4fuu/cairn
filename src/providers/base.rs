use crate::model::{MessageRecord, ParsedSession, SessionRecord, ToolCallRecord, ToolResultRecord};
use crate::util::{
    clip, content_text, first_line, is_prompt_text, json_string, normalize_ts, project_name,
    stable_id, MAX_TOOL_INPUT, MAX_TOOL_OUTPUT,
};
use serde_json::Value;
use std::collections::HashSet;
use std::path::Path;

#[derive(Clone, Copy, Default, Debug)]
pub struct Usage {
    pub input: Option<i64>,
    pub output: Option<i64>,
    pub cache_read: Option<i64>,
    pub cache_write: Option<i64>,
}

pub struct SessionBuilder {
    pub session: SessionRecord,
    messages: Vec<MessageRecord>,
    calls: Vec<ToolCallRecord>,
    results: Vec<ToolResultRecord>,
    warnings: Vec<String>,
    seq: i64,
    uuids: HashSet<String>,
    first_prompt: Option<String>,
    start: Option<String>,
    end: Option<String>,
}

impl SessionBuilder {
    pub fn new(source: &str, path: &Path) -> Self {
        let stem = path
            .file_stem()
            .and_then(|name| name.to_str())
            .unwrap_or("session")
            .to_string();
        let id = if source == "claude" {
            stem.clone()
        } else {
            format!("{source}:{stem}")
        };
        Self {
            session: SessionRecord {
                id,
                source: source.to_string(),
                source_path: path.to_string_lossy().to_string(),
                ..Default::default()
            },
            messages: Vec::new(),
            calls: Vec::new(),
            results: Vec::new(),
            warnings: Vec::new(),
            seq: 0,
            uuids: HashSet::new(),
            first_prompt: None,
            start: None,
            end: None,
        }
    }

    pub fn set_id(&mut self, id: &str) {
        let prefix = if self.session.source == "claude" {
            String::new()
        } else {
            format!("{}:", self.session.source)
        };
        self.session.id = format!("{prefix}{id}");
    }

    pub fn warn(&mut self, warning: impl Into<String>) {
        if self.warnings.len() < 20 {
            self.warnings.push(warning.into());
        }
    }

    pub fn add_message(
        &mut self,
        uuid: Option<&str>,
        role: &str,
        text: Option<String>,
        content_type: &str,
        parent_uuid: Option<&str>,
        timestamp: Option<&str>,
        is_meta: bool,
        model: Option<&str>,
        usage: Usage,
        agent_id: Option<&str>,
        extra: Option<Value>,
    ) -> String {
        let mut id = uuid
            .map(ToString::to_string)
            .unwrap_or_else(|| format!("{}:{}", self.session.id, self.seq + 1));
        if self.uuids.contains(&id) {
            let mut suffix = 2;
            while self.uuids.contains(&format!("{id}:{suffix}")) {
                suffix += 1;
            }
            id = format!("{id}:{suffix}");
        }
        self.uuids.insert(id.clone());
        self.seq += 1;
        let stamp = timestamp.and_then(|value| normalize_ts(&Value::String(value.to_string())));
        if let Some(stamp) = &stamp {
            if self.start.as_ref().is_none_or(|start| stamp < start) {
                self.start = Some(stamp.clone());
            }
            if self.end.as_ref().is_none_or(|end| stamp > end) {
                self.end = Some(stamp.clone());
            }
        }
        if !is_meta && role == "user" {
            if let Some(text) = text.as_deref() {
                if is_prompt_text(text) && self.first_prompt.is_none() {
                    self.first_prompt = first_line(text);
                }
            }
        }
        let extra_json = extra
            .as_ref()
            .map(json_string)
            .filter(|value| value != "null");
        self.messages.push(MessageRecord {
            uuid: id.clone(),
            seq: self.seq,
            role: role.to_string(),
            content_type: content_type.to_string(),
            text,
            parent_uuid: parent_uuid.map(ToString::to_string),
            timestamp: stamp,
            is_meta,
            model: model.map(ToString::to_string),
            input_tokens: usage.input,
            output_tokens: usage.output,
            cache_read_tokens: usage.cache_read,
            cache_write_tokens: usage.cache_write,
            agent_id: agent_id.map(ToString::to_string),
            extra_json,
        });
        id
    }

    #[allow(clippy::too_many_arguments)]
    pub fn add_tool_use(
        &mut self,
        uuid: &str,
        parent_uuid: Option<&str>,
        name: &str,
        arguments: &Value,
        call_id: Option<&str>,
        timestamp: Option<&str>,
        is_meta: bool,
        agent_id: Option<&str>,
    ) -> String {
        let input_json = json_string(arguments);
        let input_json = clip(&input_json, MAX_TOOL_INPUT).0;
        let message_uuid = self.add_message(
            Some(uuid),
            "assistant",
            Some(name.to_string()),
            "tool_use",
            parent_uuid,
            timestamp,
            is_meta,
            None,
            Usage::default(),
            agent_id,
            Some(serde_json::json!({"tool_name": name, "input": arguments})),
        );
        let id = call_id
            .map(ToString::to_string)
            .unwrap_or_else(|| format!("{}:call:{}", self.session.id, self.calls.len() + 1));
        self.calls.push(ToolCallRecord {
            id,
            message_uuid: message_uuid.clone(),
            name: name.to_string(),
            input_json,
            file_path: tool_file_path(arguments),
            timestamp: timestamp.map(ToString::to_string),
        });
        message_uuid
    }

    #[allow(clippy::too_many_arguments)]
    pub fn add_tool_result(
        &mut self,
        tool_call_id: &str,
        uuid: &str,
        parent_uuid: Option<&str>,
        content: Option<String>,
        timestamp: Option<&str>,
        is_error: bool,
    ) -> String {
        let content = content.unwrap_or_default();
        let clipped = clip(&content, MAX_TOOL_OUTPUT).0;
        let message_uuid = self.add_message(
            Some(uuid),
            "tool",
            Some(clipped.clone()),
            "tool_result",
            parent_uuid,
            timestamp,
            true,
            None,
            Usage::default(),
            None,
            Some(serde_json::json!({"tool_call_id": tool_call_id, "is_error": is_error})),
        );
        self.results.push(ToolResultRecord {
            tool_call_id: tool_call_id.to_string(),
            message_uuid: message_uuid.clone(),
            content: clipped,
            is_error,
            timestamp: timestamp.map(ToString::to_string),
        });
        message_uuid
    }

    pub fn set_last_usage(&mut self, uuid: &str, usage: Usage) {
        if let Some(message) = self
            .messages
            .iter_mut()
            .rev()
            .find(|message| message.uuid == uuid)
        {
            if usage.input.is_some() {
                message.input_tokens = usage.input;
            }
            if usage.output.is_some() {
                message.output_tokens = usage.output;
            }
            if usage.cache_read.is_some() {
                message.cache_read_tokens = usage.cache_read;
            }
            if usage.cache_write.is_some() {
                message.cache_write_tokens = usage.cache_write;
            }
        }
    }

    pub fn set_project(&mut self, cwd: Option<&str>) {
        self.session.project_path = cwd.map(ToString::to_string);
        self.session.project = cwd.and_then(project_name);
    }

    pub fn finish(mut self) -> ParsedSession {
        if self.session.title.is_none() {
            self.session.title = self.first_prompt.take();
        }
        if let Some(start) = self.start {
            if self
                .session
                .started_at
                .as_ref()
                .is_none_or(|value| &start < value)
            {
                self.session.started_at = Some(start);
            }
        }
        if let Some(end) = self.end {
            if self
                .session
                .ended_at
                .as_ref()
                .is_none_or(|value| &end > value)
            {
                self.session.ended_at = Some(end);
            }
        }
        ParsedSession {
            session: self.session,
            messages: self.messages,
            tool_calls: self.calls,
            tool_results: self.results,
            warnings: self.warnings,
        }
    }
}

pub fn tool_file_path(value: &Value) -> Option<String> {
    let map = value.as_object()?;
    for key in [
        "file_path",
        "filePath",
        "path",
        "notebook_path",
        "target_file",
    ] {
        if let Some(Value::String(path)) = map.get(key) {
            if !path.trim().is_empty() {
                return Some(path.trim().to_string());
            }
        }
    }
    for key in ["command", "cmd"] {
        if let Some(Value::String(command)) = map.get(key) {
            if let Some(path) = absolute_path_in(command) {
                return Some(path);
            }
        }
    }
    None
}

fn absolute_path_in(command: &str) -> Option<String> {
    for token in command.split_whitespace() {
        let token = token.trim_matches(|c: char| "'\";|&()[]{}".contains(c));
        if token.starts_with('/') && token.len() > 1 {
            return Some(token.to_string());
        }
    }
    None
}

pub fn value_text(value: &Value) -> Option<String> {
    content_text(value)
}

pub fn simple_uuid(parts: &[&str]) -> String {
    stable_id(parts)
}
