use arbitrary::Arbitrary;
use facet::Facet;

/// Launch the Windows tray UI for daemon log replay and live follow.
#[derive(Facet, Arbitrary, PartialEq, Debug, Default)]
pub struct TrayArgs;

impl TrayArgs {
    /// # Errors
    ///
    /// Returns an error if the tray UI cannot be initialized.
    pub fn invoke(self) -> eyre::Result<()> {
        let context = crate::tray::TrayContext {
            inherited_console_available: teamy_windows::console::is_inheriting_console(),
        };
        crate::tray::run_tray(context)
    }
}
