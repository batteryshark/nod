use anyhow::Result;

use super::{RuntimeCommand, RuntimeCommandOutcome, RuntimePort};

pub(crate) async fn execute_runtime_command(
    runtime: &mut impl RuntimePort,
    command: RuntimeCommand,
) -> Result<RuntimeCommandOutcome> {
    match command {
        RuntimeCommand::Enroll(params) => {
            let mut state = runtime.enroll(params).await?;
            // Enrollment returns a state snapshot; starting sync here matches
            // the startup path that already-registered devices use.
            if let Err(error) = runtime.connect_sync().await {
                state.last_error = Some(format!(
                    "Enrollment completed. Sync could not start: {error}"
                ));
            }
            Ok(RuntimeCommandOutcome::State(Box::new(state)))
        }
        RuntimeCommand::Refresh => runtime
            .refresh()
            .await
            .map(Box::new)
            .map(RuntimeCommandOutcome::State),
        RuntimeCommand::ConnectSync => {
            runtime.connect_sync().await?;
            Ok(RuntimeCommandOutcome::None)
        }
        RuntimeCommand::SelectServer(params) => runtime
            .select_server(params)
            .await
            .map(Box::new)
            .map(RuntimeCommandOutcome::State),
        RuntimeCommand::ForgetServer(params) => runtime
            .forget_server(params)
            .await
            .map(Box::new)
            .map(RuntimeCommandOutcome::State),
        RuntimeCommand::SelectChannel(params) => runtime
            .select_channel(params)
            .await
            .map(Box::new)
            .map(RuntimeCommandOutcome::State),
        RuntimeCommand::QueryHistory(query) => {
            let page = runtime.query_history(query.clone()).await?;
            Ok(RuntimeCommandOutcome::History { query, page })
        }
        RuntimeCommand::SelectAllChannels => runtime
            .select_all_channels()
            .await
            .map(Box::new)
            .map(RuntimeCommandOutcome::State),
        RuntimeCommand::SelectRequest(params) => runtime
            .select_request(params)
            .await
            .map(Box::new)
            .map(RuntimeCommandOutcome::State),
        RuntimeCommand::SubmitOption(params) => runtime
            .submit_option(params)
            .await
            .map(Box::new)
            .map(RuntimeCommandOutcome::Request),
        RuntimeCommand::ClearChannel(params) => runtime
            .clear_channel(params)
            .await
            .map(Box::new)
            .map(RuntimeCommandOutcome::State),
        RuntimeCommand::SetSubscription(params) => runtime
            .set_subscription(params)
            .await
            .map(Box::new)
            .map(RuntimeCommandOutcome::State),
        RuntimeCommand::SetNotificationPreference(params) => runtime
            .set_notification_preference(params)
            .await
            .map(Box::new)
            .map(RuntimeCommandOutcome::State),
        RuntimeCommand::ListDevices => runtime
            .list_devices()
            .await
            .map(RuntimeCommandOutcome::Devices),
        RuntimeCommand::RenameDevice(params) => runtime
            .rename_device(params)
            .await
            .map(Box::new)
            .map(RuntimeCommandOutcome::Device),
        RuntimeCommand::RevokeDevice(params) => runtime
            .revoke_device(params)
            .await
            .map(Box::new)
            .map(RuntimeCommandOutcome::State),
    }
}
