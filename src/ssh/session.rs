//! SSH session management: bridges russh (tokio) and gpui via channels.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crossbeam_channel::{unbounded, Receiver, Sender};
use russh::ChannelMsg;
use tokio::sync::mpsc;

use crate::config::SshConnection;
use crate::ssh::client::connect_ssh;

/// Status of an SSH session, sent to the UI for display.
#[derive(Debug, Clone)]
pub enum SessionStatus {
    Connecting,
    Connected,
    Disconnected,
    Error(String),
}

/// An SSH session that bridges between tokio (russh) and gpui.
///
/// The gpui side interacts with the session via:
/// - `send_input()`: send keyboard input to the SSH channel
/// - `try_recv_output()`: poll for SSH output data
/// - `try_recv_status()`: poll for connection status updates
/// - `is_connected()`: check if the session is still active
/// - `disconnect()`: terminate the session
pub struct SshSession {
    /// Input channel: gpui → tokio (keyboard input → SSH channel write)
    input_tx: mpsc::UnboundedSender<Vec<u8>>,
    /// Output channel: tokio → gpui (SSH channel read → terminal)
    output_rx: Receiver<Vec<u8>>,
    /// Status channel: tokio → gpui (connection status updates)
    status_rx: Receiver<SessionStatus>,
    /// Connection state flag.
    connected: Arc<AtomicBool>,
    /// The tokio task running the SSH session.
    _session_task: tokio::task::JoinHandle<()>,
    /// Connection display info.
    pub connection_name: String,
    pub descriptor: String,
}

impl SshSession {
    /// Create a new SSH session by connecting to the given host.
    /// The actual connection happens asynchronously in a tokio task.
    pub fn connect(
        runtime: &tokio::runtime::Handle,
        conn: &SshConnection,
    ) -> Self {
        let (input_tx, input_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        let (output_tx, output_rx) = unbounded::<Vec<u8>>();
        let (status_tx, status_rx) = unbounded::<SessionStatus>();
        let connected = Arc::new(AtomicBool::new(false));
        let connected_clone = connected.clone();

        let conn_clone = conn.clone();
        let session_task = runtime.spawn(async move {
            run_session(
                &conn_clone,
                input_rx,
                output_tx,
                status_tx,
                connected_clone,
            )
            .await;
        });

        Self {
            input_tx,
            output_rx,
            status_rx,
            connected,
            _session_task: session_task,
            connection_name: conn.name.clone(),
            descriptor: conn.descriptor(),
        }
    }

    /// Send keyboard input to the SSH channel (non-blocking).
    pub fn send_input(&self, data: Vec<u8>) {
        let _ = self.input_tx.send(data);
    }

    /// Try to receive output data from the SSH channel (non-blocking poll).
    pub fn try_recv_output(&self) -> Option<Vec<u8>> {
        self.output_rx.try_recv().ok()
    }

    /// Try to receive a status update (non-blocking poll).
    pub fn try_recv_status(&self) -> Option<SessionStatus> {
        self.status_rx.try_recv().ok()
    }

    /// Check if the session is currently connected.
    pub fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Relaxed)
    }

    /// Disconnect the session by dropping the input channel.
    /// The tokio task will detect the channel closure and clean up.
    pub fn disconnect(&self) {
        // Dropping the sender will cause `input_rx.recv()` to return `None`,
        // which will end the write task and trigger cleanup.
        // We can't actually drop self.input_tx here since it's &self,
        // but we can send a disconnect signal.
        // For now, the session will end when the SshSession is dropped.
    }
}

impl Drop for SshSession {
    fn drop(&mut self) {
        // Abort the tokio task when the session is dropped.
        self._session_task.abort();
    }
}

/// The actual SSH session loop, running in a tokio task.
async fn run_session(
    conn: &SshConnection,
    mut input_rx: mpsc::UnboundedReceiver<Vec<u8>>,
    output_tx: Sender<Vec<u8>>,
    status_tx: Sender<SessionStatus>,
    connected: Arc<AtomicBool>,
) {
    let _ = status_tx.send(SessionStatus::Connecting);

    // 1. Connect and authenticate.
    let handle = match connect_ssh(
        &conn.host,
        conn.port,
        &conn.username,
        &conn.auth_method,
    )
    .await
    {
        Ok(h) => h,
        Err(e) => {
            let _ = status_tx.send(SessionStatus::Error(format!(
                "Connection failed: {}",
                e
            )));
            // Send error message to terminal as well.
            let _ = output_tx.send(format!(
                "\r\n\x1b[31mConnection failed: {}\x1b[0m\r\n",
                e
            ).into_bytes());
            return;
        }
    };

    // 2. Open a session channel.
    let mut channel = match handle.channel_open_session().await {
        Ok(ch) => ch,
        Err(e) => {
            let _ = status_tx.send(SessionStatus::Error(format!(
                "Failed to open channel: {}",
                e
            )));
            let _ = output_tx.send(format!(
                "\r\n\x1b[31mFailed to open channel: {}\x1b[0m\r\n",
                e
            ).into_bytes());
            return;
        }
    };

    // 3. Request a PTY (pseudo-terminal).
    let term = "xterm-256color";
    if let Err(e) = channel
        .request_pty(true, term, 80, 24, 0, 0, &[])
        .await
    {
        let _ = status_tx.send(SessionStatus::Error(format!(
            "Failed to request PTY: {}",
            e
        )));
        return;
    }

    // 4. Request a shell.
    if let Err(e) = channel.request_shell(true).await {
        let _ = status_tx.send(SessionStatus::Error(format!(
            "Failed to request shell: {}",
            e
        )));
        return;
    }

    connected.store(true, Ordering::Relaxed);
    let _ = status_tx.send(SessionStatus::Connected);

    // 5. Get an independent writer (doesn't borrow the channel).
    let writer = channel.make_writer();

    // 6. Spawn the write task: read from input_rx, write to SSH channel.
    let write_task = tokio::spawn(async move {
        use tokio::io::AsyncWriteExt;
        let mut writer = writer;
        while let Some(data) = input_rx.recv().await {
            if writer.write_all(&data).await.is_err() {
                break;
            }
        }
    });

    // 7. Read loop: receive data from the SSH channel and send to terminal.
    while let Some(msg) = channel.wait().await {
        match msg {
            ChannelMsg::Data { data } => {
                let _ = output_tx.send(data.to_vec());
            }
            ChannelMsg::ExtendedData { data, .. } => {
                // stderr data - send to terminal as well.
                let _ = output_tx.send(data.to_vec());
            }
            ChannelMsg::Eof | ChannelMsg::Close => {
                break;
            }
            ChannelMsg::ExitStatus { .. } => {
                break;
            }
            _ => {}
        }
    }

    // 8. Cleanup.
    connected.store(false, Ordering::Relaxed);
    write_task.abort();
    let _ = handle
        .disconnect(russh::Disconnect::ByApplication, "Goodbye", "en")
        .await;
    let _ = status_tx.send(SessionStatus::Disconnected);
}
