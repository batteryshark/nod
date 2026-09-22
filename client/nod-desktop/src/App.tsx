import { useState } from "react";
import { Modal } from "./components/Modal";
import { useDesktopClient } from "./app/useDesktopClient";
import { EnrollmentView } from "./components/EnrollmentView";
import { RequestDetail } from "./components/RequestDetail";
import { RequestList } from "./components/RequestList";
import { SettingsDialog } from "./components/SettingsDialog";
import { Sidebar } from "./components/Sidebar";
import { Topbar } from "./components/Topbar";

export function App(): JSX.Element {
  const client = useDesktopClient();
  const [addingServer, setAddingServer] = useState(false);

  if (client.isLoading) {
    return <div className="boot">Nod</div>;
  }

  if (!client.state.is_registered) {
    return (
      <EnrollmentView
        error={client.error}
        onEnroll={client.commands.enrollDevice}
      />
    );
  }

  return (
    <div className="shell">
      <Sidebar
        activeChannel={client.activeChannel}
        onAddServer={() => setAddingServer(true)}
        onSelectAll={client.commands.selectAllChannels}
        onOpenSettings={client.commands.openSettings}
        onRefresh={client.commands.refreshState}
        onSelectChannel={client.commands.selectChannel}
        onSelectServer={client.commands.selectServer}
        state={client.state}
      />
      <main className="workbench">
        <Topbar
          activeChannel={client.activeChannel}
          error={client.error}
          phase={client.state.sync_phase}
          lastSyncedAt={client.state.last_synced_at}
          onRetry={client.commands.refreshState}
          onDismissError={client.commands.clearError}
        />
        <section className="columns">
          <RequestList
            key={`${client.state.selected_server_id}:${client.activeChannel?.id ?? "all"}`}
            serverId={client.state.selected_server_id ?? ""}
            channelId={client.activeChannel?.id}
            requests={client.state.requests}
            selectedRequestId={client.activeRequest?.id ?? null}
            onSelect={client.commands.selectRequest}
          />
          <RequestDetail
            request={client.activeRequest}
            serverId={client.state.selected_server_id ?? undefined}
            allowRemoteImages={client.preferences.load_remote_images}
            onOption={client.commands.submitRequestOption}
            onOpenUrl={client.commands.openUrl}
          />
        </section>
      </main>
      {addingServer ? (
        <Modal title="Add server" onClose={() => setAddingServer(false)}>
          <EnrollmentView
            error={client.error}
            onEnroll={client.commands.enrollDevice}
            onCancel={() => setAddingServer(false)}
          />
        </Modal>
      ) : null}
      {client.settingsOpen ? (
        <SettingsDialog
          commands={client.commands}
          devices={client.devices}
          preferences={client.preferences}
          autostart={client.autostart}
          error={client.error}
          state={client.state}
        />
      ) : null}
    </div>
  );
}
