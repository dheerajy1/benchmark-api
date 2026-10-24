use serde_json::{Value, json};

// Where a live run came from. Only the target id and name are recorded, never a header value
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunOrigin {
    // A raw request that names the command and the path itself
    Custom,
    // A run started from a saved target
    Target { id: String, name: String },
}

impl RunOrigin {
    // The source, target id and target name as they appear in run.accepted
    pub fn event_fields(&self) -> (&'static str, Value, Value) {
        match self {
            RunOrigin::Custom => ("custom", Value::Null, Value::Null),
            RunOrigin::Target { id, name } => ("target", json!(id), json!(name)),
        }
    }
}
