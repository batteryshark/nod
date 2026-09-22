mod alerts;
mod app;
mod domain;
mod runtime_bridge;
#[cfg(test)]
mod smoke_test;
mod terminal;
#[cfg(test)]
mod test_support;
mod ui;

use std::sync::Arc;

use anyhow::Result;
use nod_client_core::{NodClientMessage, NodClientRuntime};
use tokio::sync::{mpsc, Mutex};

use crate::{
    app::AppState,
    runtime_bridge::{execute_runtime_command, RuntimeCommand, RuntimeCommandOutcome},
    terminal::TerminalSession,
};

const RUNTIME_MESSAGE_CAPACITY: usize = 128;
const COMMAND_RESULT_CAPACITY: usize = 16;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .without_time()
        .init();

    run().await
}

async fn run() -> Result<()> {
    let (message_tx, message_rx) = mpsc::channel::<NodClientMessage>(RUNTIME_MESSAGE_CAPACITY);
    let runtime = NodClientRuntime::new(message_tx).await?;
    let app = AppState::new(runtime.state().await);
    let runtime = Arc::new(Mutex::new(runtime));
    let terminal = TerminalSession::enter()?;
    let (command_tx, command_rx) = mpsc::channel::<CommandResultMessage>(COMMAND_RESULT_CAPACITY);

    runtime.lock().await.emit_ready().await;
    let startup_commands = startup_commands_for(&app);
    run_message_loop(
        runtime,
        app,
        terminal,
        message_rx,
        command_tx,
        command_rx,
        startup_commands,
    )
    .await
}

async fn run_message_loop(
    runtime: SharedRuntime,
    mut app: AppState,
    mut terminal: TerminalSession,
    mut message_rx: mpsc::Receiver<NodClientMessage>,
    command_tx: mpsc::Sender<CommandResultMessage>,
    mut command_rx: mpsc::Receiver<CommandResultMessage>,
    startup_commands: Vec<RuntimeCommand>,
) -> Result<()> {
    let mut in_flight_commands = 0usize;

    for command in startup_commands {
        if start_command(&runtime, &command_tx, &mut app, command) {
            in_flight_commands += 1;
        }
    }

    while !app.should_quit() {
        drain_runtime_messages(&mut app, &mut terminal, &mut message_rx)?;
        in_flight_commands =
            in_flight_commands.saturating_sub(drain_command_results(&mut app, &mut command_rx));
        app.tick();
        terminal.draw(&app)?;

        let Some(key) = terminal.read_key()? else {
            continue;
        };
        let commands = app.handle_key(key);
        for command in commands {
            // One runtime command at a time: while one is in flight, the rest
            // of this keypress's commands are dropped (not queued) — queuing
            // them would replay stale intent against whatever state the
            // in-flight command produces.
            if in_flight_commands > 0 {
                app.set_busy_notice();
                break;
            }
            if start_command(&runtime, &command_tx, &mut app, command) {
                in_flight_commands += 1;
            }
        }
    }

    if let Ok(mut runtime) = runtime.try_lock() {
        runtime.disconnect_sync().await;
    }
    Ok(())
}

fn drain_runtime_messages(
    app: &mut AppState,
    terminal: &mut TerminalSession,
    message_rx: &mut mpsc::Receiver<NodClientMessage>,
) -> Result<()> {
    while let Ok(message) = message_rx.try_recv() {
        if app.apply_runtime_message(message).ring_bell {
            terminal.ring_bell()?;
        }
    }
    Ok(())
}

type SharedRuntime = Arc<Mutex<NodClientRuntime>>;

struct CommandResultMessage {
    result: std::result::Result<RuntimeCommandOutcome, String>,
}

fn startup_commands_for(app: &AppState) -> Vec<RuntimeCommand> {
    if app.is_registered() {
        vec![RuntimeCommand::Refresh, RuntimeCommand::ConnectSync]
    } else {
        Vec::new()
    }
}

fn start_command(
    runtime: &SharedRuntime,
    command_tx: &mpsc::Sender<CommandResultMessage>,
    app: &mut AppState,
    command: RuntimeCommand,
) -> bool {
    tracing::debug!(command = command.label(), "running runtime command");
    app.begin_command(&command);
    let runtime = Arc::clone(runtime);
    let command_tx = command_tx.clone();

    tokio::spawn(async move {
        let result = {
            let mut runtime = runtime.lock().await;
            execute_runtime_command(&mut *runtime, command)
                .await
                .map_err(|error| error.to_string())
        };
        let _ = command_tx.send(CommandResultMessage { result }).await;
    });

    true
}

fn drain_command_results(
    app: &mut AppState,
    command_rx: &mut mpsc::Receiver<CommandResultMessage>,
) -> usize {
    let mut drained = 0;
    while let Ok(message) = command_rx.try_recv() {
        drained += 1;
        match message.result {
            Ok(outcome) => app.apply_runtime_outcome(outcome),
            Err(message) => {
                tracing::warn!(error = %message, "runtime command failed");
                app.set_error(message);
            }
        }
    }
    drained
}
