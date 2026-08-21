use super::*;

impl CodeTerminalManager {
    /// Resize one session only when its complete owner identity matches.
    pub fn resize(&self, input: CodeTerminalResizeInput) -> Result<(), String> {
        validate_dimensions(input.cols, input.rows)?;
        let owner = owner_for_control(&input.scope, &input.thread_id, &input.session_id)?;
        let reply = send_control(&self.inner, &owner, &input.session_id, |reply| {
            SessionControl::Resize {
                cols: input.cols,
                rows: input.rows,
                reply,
            }
        })?;
        wait_for_reply(reply, "resize")
    }

    /// Write raw bytes to one exact native terminal owner.
    pub fn stdin(&self, input: CodeTerminalStdinInput) -> Result<(), String> {
        if input.data.len() > MAX_STDIN_BYTES {
            return Err(format!(
                "SchoolX Code terminal stdin exceeds the {MAX_STDIN_BYTES}-byte limit"
            ));
        }
        let owner = owner_for_control(&input.scope, &input.thread_id, &input.session_id)?;
        let reply = send_control(&self.inner, &owner, &input.session_id, |reply| {
            SessionControl::Stdin {
                data: input.data,
                reply,
            }
        })?;
        wait_for_reply(reply, "stdin")
    }

    /// Terminate one session only when its complete owner identity matches.
    pub fn terminate(&self, input: CodeTerminalTerminateInput) -> Result<(), String> {
        let owner = owner_for_control(&input.scope, &input.thread_id, &input.session_id)?;
        let reply_rx = {
            let mut inner = lock_manager(&self.inner)?;
            let Some(entry) = inner.sessions.get_mut(&input.session_id) else {
                return Err("SchoolX Code terminal session was not found".to_string());
            };
            ensure_exact_owner(&entry.owner, &owner)?;
            if entry.closing {
                return Ok(());
            }
            entry.closing = true;
            let (reply_tx, reply_rx) = mpsc::sync_channel(1);
            let sent = entry.terminate_tx.send(TerminateControl {
                reply: Some(reply_tx),
            });
            if sent.is_err() {
                inner.sessions.remove(&input.session_id);
                return Err("SchoolX Code terminal actor is unavailable".to_string());
            }
            reply_rx
        };
        wait_for_reply(reply_rx, "terminate")
    }
}
