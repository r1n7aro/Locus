//! Named-pipe server on the broker's own tokio thread. Every connection gets
//! its own task, so several Locus processes (for example a development build
//! and the installed app) can talk to one host at the same time. A new
//! instance is created as soon as a client connects, keeping the name
//! connectable while other clients are served.

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::windows::named_pipe::{NamedPipeServer, PipeMode, ServerOptions};

use crate::broker::{handle_line, Broker};
use crate::pipe_security::PipeSecurity;

const ERROR_ACCESS_DENIED: i32 = 5;

fn create_server(
    pipe_name: &str,
    first: bool,
    security: &mut Option<PipeSecurity>,
) -> std::io::Result<NamedPipeServer> {
    let mut options = ServerOptions::new();
    options
        .access_inbound(true)
        .access_outbound(true)
        .pipe_mode(PipeMode::Byte)
        .first_pipe_instance(first);
    match security.as_mut() {
        // SAFETY: the attributes and their descriptor outlive the call.
        Some(security) => unsafe { options.create_with_security_attributes_raw(pipe_name, security.as_raw()) },
        None => options.create(pipe_name),
    }
}

async fn serve_connection(broker: Arc<Broker>, server: NamedPipeServer) {
    let (read_half, mut write_half) = tokio::io::split(server);
    let (client, outbox) = broker.register_client();
    eprintln!(
        "[locus-native] client {client} connected: {}",
        broker.config.pipe_name
    );

    let writer_outbox = Arc::clone(&outbox);
    let writer = tokio::spawn(async move {
        while let Some(bytes) = writer_outbox.next().await {
            let written = write_half.write_all(&bytes).await.is_ok() && write_half.flush().await.is_ok();
            writer_outbox.written();
            if !written {
                break;
            }
        }
    });

    let mut reader = BufReader::new(read_half);
    let mut buf: Vec<u8> = Vec::with_capacity(4096);
    let mut authenticated = !broker.requires_hello();
    loop {
        buf.clear();
        let read = tokio::select! {
            _ = broker.shutdown_notify.notified() => break,
            // The broker disconnected a client too far behind on replies.
            _ = outbox.closed() => break,
            r = reader.read_until(b'\n', &mut buf) => r,
        };
        match read {
            Ok(0) => break, // client closed
            Ok(_) if authenticated => handle_line(&broker, client, &buf),
            Ok(_) => authenticated = broker.authenticate(client, &buf),
            Err(e) => {
                eprintln!("[locus-native] pipe read error: {e}");
                break;
            }
        }
    }

    // Closing the outbox ends an idle writer; a writer blocked on a client
    // that stopped reading is cancelled, which closes the pipe instance.
    broker.unregister_client(client);
    writer.abort();
    let _ = writer.await;
    eprintln!(
        "[locus-native] client {client} disconnected: {}",
        broker.config.pipe_name
    );
}

pub(crate) async fn broker_main(broker: Arc<Broker>) {
    let pipe_name = broker.config.pipe_name.clone();
    eprintln!("[locus-native] broker listening on {pipe_name}");
    let mut security = PipeSecurity::current_user_only();
    if security.is_none() {
        eprintln!("[locus-native] current-user pipe DACL unavailable; using the default descriptor");
    }
    // The first instance claims the name; a process that created it earlier
    // (a squatter, or a previous broker still closing) makes this fail.
    let mut first = true;
    loop {
        if broker.shutdown.load(Ordering::SeqCst) {
            break;
        }

        let server = match create_server(&pipe_name, first, &mut security) {
            Ok(server) => {
                first = false;
                server
            }
            Err(e) => {
                if first && e.raw_os_error() == Some(ERROR_ACCESS_DENIED) {
                    eprintln!("[locus-native] pipe name is held by another instance, retrying: {pipe_name}");
                } else {
                    eprintln!("[locus-native] create pipe failed: {e}");
                }
                tokio::select! {
                    _ = broker.shutdown_notify.notified() => break,
                    _ = tokio::time::sleep(Duration::from_millis(500)) => continue,
                }
            }
        };

        tokio::select! {
            _ = broker.shutdown_notify.notified() => break,
            connect = server.connect() => match connect {
                Ok(()) => {
                    tokio::spawn(serve_connection(Arc::clone(&broker), server));
                }
                Err(e) => {
                    eprintln!("[locus-native] wait-for-connection failed: {e}");
                    tokio::time::sleep(Duration::from_millis(200)).await;
                }
            },
        }
    }
    eprintln!("[locus-native] broker loop exited");
}
