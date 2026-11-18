//! Small helpers shared by the finite native integration journeys.

use egui_dockspace_native::{NativeActionReceipt, NativeActionStatus};

/// Polls the receipt owned by one journey, consuming it at the terminal boundary.
///
/// A disconnected sender is an explicit failure, not another pending frame.
pub fn poll_action_receipt(
    receipt: &mut Option<NativeActionReceipt>,
) -> Option<Result<NativeActionStatus, String>> {
    let result = match receipt.as_ref()?.try_recv() {
        Ok(result) => result.map_err(|error| error.to_string()),
        Err(std::sync::mpsc::TryRecvError::Empty) => return None,
        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
            Err("the native runtime ended before this operation completed".to_owned())
        }
    };
    *receipt = None;
    Some(result)
}
