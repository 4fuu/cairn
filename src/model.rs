use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SessionRecord {
    pub id: String,
    pub source: String,
    pub title: Option<String>,
    pub project: Option<String>,
    pub project_path: Option<String>,
    pub started_at: Option<String>,
    pub ended_at: Option<String>,
    pub git_branch: Option<String>,
    pub model: Option<String>,
    pub version: Option<String>,
    pub summary: Option<String>,
    pub source_path: String,
    pub meta_json: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct MessageRecord {
    pub uuid: String,
    pub seq: i64,
    pub role: String,
    pub content_type: String,
    pub text: Option<String>,
    pub parent_uuid: Option<String>,
    pub timestamp: Option<String>,
    pub is_meta: bool,
    pub model: Option<String>,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub cache_read_tokens: Option<i64>,
    pub cache_write_tokens: Option<i64>,
    pub agent_id: Option<String>,
    pub extra_json: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ToolCallRecord {
    pub id: String,
    pub message_uuid: String,
    pub name: String,
    pub input_json: String,
    pub file_path: Option<String>,
    pub timestamp: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ToolResultRecord {
    pub tool_call_id: String,
    pub message_uuid: String,
    pub content: String,
    pub is_error: bool,
    pub timestamp: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct ParsedSession {
    pub session: SessionRecord,
    pub messages: Vec<MessageRecord>,
    pub tool_calls: Vec<ToolCallRecord>,
    pub tool_results: Vec<ToolResultRecord>,
    pub warnings: Vec<String>,
}
