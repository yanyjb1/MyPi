//! Number and money formats owned by the Main zone.

use crate::ui::tui::zone::main::input::Lang;

mod compact;
mod exact;
mod money;

pub use compact::compact;
pub use exact::exact;
pub use money::money;
