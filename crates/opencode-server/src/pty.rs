use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{Read as _, Write as _};
use std::sync::{Arc, Mutex};
use tokio::sync::{broadcast, RwLock};
use tokio::task::JoinHandle;

/// Maximum size of the retained output buffer (2 MiB, matching TS).
const BUFFER_LIMIT: usize = 2 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PtySession {
    pub id: String,
    pub command: String,
    pub cwd: String,
    pub status: PtyStatus,
    pub env: HashMap<String, String>,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum PtyStatus {
    Running,
    Exited,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PtyOutput {
    pub session_id: String,
    pub data: String,
    pub is_error: bool,
}

struct PtySessionInner {
    master: Mutex<Box<dyn MasterPty + Send>>,
    writer: Arc<Mutex<Box<dyn std::io::Write + Send>>>,
    child: Mutex<Box<dyn portable_pty::Child + Send>>,
    /// Retained output buffer for replay on WebSocket connect.
    output_buffer: Arc<Mutex<Vec<u8>>>,
    /// Cumulative byte count of all output ever produced (including trimmed bytes).
    /// Used together with `output_buffer` to compute replay offsets.
    cursor: Arc<Mutex<usize>>,
    /// Broadcast channel for live PTY output to WebSocket subscribers.
    output_tx: broadcast::Sender<Vec<u8>>,
    reader_handle: JoinHandle<()>,
}

pub struct PtyManager {
    sessions: Arc<RwLock<HashMap<String, (PtySession, PtySessionInner)>>>,
}

impl PtyManager {
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn create_session(
        &self,
        command: &str,
        cwd: Option<&str>,
        env: Option<HashMap<String, String>>,
    ) -> Result<PtySession, PtyError> {
        let id = format!("pty_{}", uuid::Uuid::new_v4().simple());
        let env_map = env.unwrap_or_default();
        let cwd_str = cwd.unwrap_or(".").to_string();

        let cmd_string = command.to_string();
        let cwd_clone = cwd_str.clone();
        let env_clone = env_map.clone();

        // Spawn the PTY on a blocking thread since portable-pty is synchronous
        let (master, child, reader) = tokio::task::spawn_blocking(move || {
            let pty_system = native_pty_system();
            let pair = pty_system
                .openpty(PtySize {
                    rows: 24,
                    cols: 80,
                    pixel_width: 0,
                    pixel_height: 0,
                })
                .map_err(|e| PtyError::SpawnFailed(e.to_string()))?;

            let mut cmd = CommandBuilder::new(&cmd_string);
            cmd.cwd(&cwd_clone);
            for (k, v) in &env_clone {
                cmd.env(k, v);
            }

            let child = pair
                .slave
                .spawn_command(cmd)
                .map_err(|e| PtyError::SpawnFailed(e.to_string()))?;

            let reader = pair
                .master
                .try_clone_reader()
                .map_err(|e| PtyError::IoError(e.to_string()))?;

            Ok::<_, PtyError>((pair.master, child, reader))
        })
        .await
        .map_err(|e| PtyError::SpawnFailed(e.to_string()))??;

        let writer = master
            .take_writer()
            .map_err(|e| PtyError::IoError(e.to_string()))?;
        let writer = Arc::new(Mutex::new(writer));

        let output_buffer: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
        let cursor: Arc<Mutex<usize>> = Arc::new(Mutex::new(0));
        let (output_tx, _) = broadcast::channel::<Vec<u8>>(256);

        let buffer_clone = output_buffer.clone();
        let cursor_clone = cursor.clone();
        let tx_clone = output_tx.clone();

        // Spawn a blocking task to continuously read PTY output, buffer it,
        // and broadcast to any connected WebSocket subscribers.
        let reader_handle = tokio::task::spawn_blocking(move || {
            let mut reader = reader;
            let mut buf = [0u8; 4096];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        let chunk = buf[..n].to_vec();
                        {
                            let mut b = buffer_clone.lock().unwrap();
                            let mut c = cursor_clone.lock().unwrap();
                            b.extend_from_slice(&chunk);
                            *c += n;
                            // Trim buffer if it exceeds the limit, matching TS behaviour.
                            if b.len() > BUFFER_LIMIT {
                                let excess = b.len() - BUFFER_LIMIT;
                                b.drain(..excess);
                            }
                        }
                        // Best-effort broadcast; receivers may lag and miss frames.
                        let _ = tx_clone.send(chunk);
                    }
                    Err(_) => break,
                }
            }
        });

        let session = PtySession {
            id: id.clone(),
            command: command.to_string(),
            cwd: cwd_str,
            status: PtyStatus::Running,
            env: env_map,
            created_at: chrono::Utc::now().timestamp(),
        };

        let inner = PtySessionInner {
            master: Mutex::new(master),
            writer,
            child: Mutex::new(child),
            output_buffer,
            cursor,
            output_tx,
            reader_handle,
        };

        self.sessions
            .write()
            .await
            .insert(id, (session.clone(), inner));

        Ok(session)
    }

    pub async fn get_session(&self, id: &str) -> Option<PtySession> {
        self.sessions.read().await.get(id).map(|(s, _)| s.clone())
    }

    pub async fn list_sessions(&self) -> Vec<PtySession> {
        self.sessions
            .read()
            .await
            .values()
            .map(|(s, _)| s.clone())
            .collect()
    }

    pub async fn update_session(
        &self,
        id: &str,
        command: Option<&str>,
        cwd: Option<&str>,
    ) -> Result<PtySession, PtyError> {
        let mut sessions = self.sessions.write().await;

        if let Some((session, _)) = sessions.get_mut(id) {
            if let Some(cmd) = command {
                session.command = cmd.to_string();
            }
            if let Some(dir) = cwd {
                session.cwd = dir.to_string();
            }
            Ok(session.clone())
        } else {
            Err(PtyError::SessionNotFound(id.to_string()))
        }
    }

    pub async fn delete_session(&self, id: &str) -> bool {
        let mut sessions = self.sessions.write().await;
        if let Some((_, inner)) = sessions.remove(id) {
            // Abort the background reader task
            inner.reader_handle.abort();
            // Kill the child process
            let _ = inner.child.lock().unwrap().kill();
            true
        } else {
            false
        }
    }

    pub async fn resize_session(&self, id: &str, cols: u16, rows: u16) -> Result<(), PtyError> {
        let sessions = self.sessions.read().await;
        let (_, inner) = sessions
            .get(id)
            .ok_or_else(|| PtyError::SessionNotFound(id.to_string()))?;

        inner
            .master
            .lock()
            .unwrap()
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| PtyError::IoError(e.to_string()))?;

        Ok(())
    }

    pub async fn write_to_session(&self, id: &str, data: &[u8]) -> Result<(), PtyError> {
        let data = data.to_vec();
        let writer = {
            let sessions = self.sessions.read().await;
            let (_, inner) = sessions
                .get(id)
                .ok_or_else(|| PtyError::SessionNotFound(id.to_string()))?;
            inner.writer.clone()
        };

        tokio::task::spawn_blocking(move || {
            let mut w = writer.lock().unwrap();
            w.write_all(&data)
                .map_err(|e| PtyError::IoError(e.to_string()))?;
            w.flush().map_err(|e| PtyError::IoError(e.to_string()))?;
            Ok::<_, PtyError>(())
        })
        .await
        .map_err(|e| PtyError::IoError(e.to_string()))??;

        Ok(())
    }

    pub async fn read_from_session(&self, id: &str) -> Result<PtyOutput, PtyError> {
        let sessions = self.sessions.read().await;
        let (_, inner) = sessions
            .get(id)
            .ok_or_else(|| PtyError::SessionNotFound(id.to_string()))?;

        let data = {
            let mut buf = inner.output_buffer.lock().unwrap();
            let bytes: Vec<u8> = buf.drain(..).collect();
            String::from_utf8_lossy(&bytes).into_owned()
        };

        Ok(PtyOutput {
            session_id: id.to_string(),
            data,
            is_error: false,
        })
    }

    /// Returns a snapshot of the current output buffer, the cumulative cursor
    /// position, a broadcast receiver for live output, and a clone of the PTY
    /// writer so the WebSocket handler can forward input.
    ///
    /// This is the primary integration point for the `/{id}/connect` WebSocket
    /// endpoint, mirroring the TS `Pty.connect` behaviour:
    ///   - replay buffered output from a caller-supplied cursor
    ///   - subscribe to live output via the broadcast channel
    ///   - write user input into the PTY
    pub async fn subscribe(&self, id: &str) -> Result<PtySubscription, PtyError> {
        let sessions = self.sessions.read().await;
        let (_, inner) = sessions
            .get(id)
            .ok_or_else(|| PtyError::SessionNotFound(id.to_string()))?;

        let (buffer_snapshot, buffer_start, cursor) = {
            let buf = inner.output_buffer.lock().unwrap();
            let cursor = *inner.cursor.lock().unwrap();
            let buffer_start = cursor - buf.len();
            (buf.clone(), buffer_start, cursor)
        };

        Ok(PtySubscription {
            buffer: buffer_snapshot,
            buffer_start,
            cursor,
            rx: inner.output_tx.subscribe(),
            writer: inner.writer.clone(),
        })
    }
}

/// Handle returned by [`PtyManager::subscribe`] containing everything needed
/// to drive a WebSocket connection for a PTY session.
pub struct PtySubscription {
    /// Snapshot of the retained output buffer at subscribe time.
    pub buffer: Vec<u8>,
    /// The cumulative byte offset where `buffer` starts (i.e. total bytes
    /// produced minus `buffer.len()`).
    pub buffer_start: usize,
    /// The cumulative byte cursor at subscribe time (end of buffer).
    pub cursor: usize,
    /// Broadcast receiver for live PTY output after the snapshot.
    pub rx: broadcast::Receiver<Vec<u8>>,
    /// Writer handle to forward WebSocket input into the PTY.
    pub writer: Arc<Mutex<Box<dyn std::io::Write + Send>>>,
}

impl Default for PtyManager {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PtyError {
    #[error("PTY session not found: {0}")]
    SessionNotFound(String),

    #[error("Failed to spawn process: {0}")]
    SpawnFailed(String),

    #[error("IO error: {0}")]
    IoError(String),
}
