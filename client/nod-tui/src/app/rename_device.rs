use crossterm::event::{KeyCode, KeyEvent};
use nod_client_core::{models::UserDevice, RenameDeviceParams};

use super::{option_text::ModalResult, RuntimeCommand, TextInput};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RenameDeviceForm {
    device_id: String,
    input: TextInput,
}

impl RenameDeviceForm {
    pub(super) fn new(device: &UserDevice) -> Self {
        Self {
            device_id: device.id.clone(),
            input: TextInput::from(device.name.clone()),
        }
    }

    pub(crate) fn input(&self) -> &TextInput {
        &self.input
    }

    pub(super) fn handle_key(&mut self, key: KeyEvent) -> ModalResult<Self> {
        if key.code == KeyCode::Esc {
            return ModalResult::closed();
        }

        if key.code == KeyCode::Enter && !self.input.value().trim().is_empty() {
            return ModalResult::submitting(
                self.clone(),
                vec![RuntimeCommand::RenameDevice(RenameDeviceParams {
                    device_id: self.device_id.clone(),
                    name: self.input.value().trim().to_string(),
                })],
            );
        }

        self.input.handle_key(key);
        ModalResult::open(self.clone())
    }
}
