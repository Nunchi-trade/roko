use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandOutputStream {
    Stdout,
    Stderr,
    System,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CommandEvent {
    Started {
        command_id: String,
        command: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cwd: Option<String>,
    },
    Output {
        command_id: String,
        stream: CommandOutputStream,
        data: String,
    },
    Exited {
        command_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        exit_code: Option<i32>,
    },
    SpawnFailed {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        command_id: Option<String>,
        command: String,
        error: String,
    },
    Cancelled {
        command_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn command_event_started_serializes() {
        let event = CommandEvent::Started {
            command_id: "cmd-1".to_string(),
            command: "cargo test".to_string(),
            cwd: Some("/work".to_string()),
        };

        assert_eq!(
            serde_json::to_value(event).unwrap(),
            json!({
                "type": "started",
                "command_id": "cmd-1",
                "command": "cargo test",
                "cwd": "/work"
            })
        );
    }

    #[test]
    fn command_event_output_serializes() {
        let event = CommandEvent::Output {
            command_id: "cmd-1".to_string(),
            stream: CommandOutputStream::Stdout,
            data: "ok\n".to_string(),
        };

        assert_eq!(
            serde_json::to_value(event).unwrap(),
            json!({
                "type": "output",
                "command_id": "cmd-1",
                "stream": "stdout",
                "data": "ok\n"
            })
        );
    }

    #[test]
    fn command_event_exited_serializes() {
        let event = CommandEvent::Exited {
            command_id: "cmd-1".to_string(),
            exit_code: Some(0),
        };

        assert_eq!(
            serde_json::to_value(event).unwrap(),
            json!({
                "type": "exited",
                "command_id": "cmd-1",
                "exit_code": 0
            })
        );
    }

    #[test]
    fn command_event_spawn_failed_serializes() {
        let event = CommandEvent::SpawnFailed {
            command_id: None,
            command: "missing-bin".to_string(),
            error: "not found".to_string(),
        };

        assert_eq!(
            serde_json::to_value(event).unwrap(),
            json!({
                "type": "spawn_failed",
                "command": "missing-bin",
                "error": "not found"
            })
        );
    }

    #[test]
    fn command_event_cancelled_serializes() {
        let event = CommandEvent::Cancelled {
            command_id: "cmd-1".to_string(),
            reason: Some("shutdown".to_string()),
        };

        assert_eq!(
            serde_json::to_value(event).unwrap(),
            json!({
                "type": "cancelled",
                "command_id": "cmd-1",
                "reason": "shutdown"
            })
        );
    }
}
