//! Names shared by the installed build's Windows and storage integrations.

use windows::core::{PCWSTR, w};

#[cfg(feature = "dev-build")]
pub const DATA_DIR: &str = "Lilt Dev";
#[cfg(not(feature = "dev-build"))]
pub const DATA_DIR: &str = "Lilt";

#[cfg(feature = "dev-build")]
pub const INSTANCE_MUTEX: PCWSTR = w!("Local\\Lilt.Dev.SingleInstance");
#[cfg(not(feature = "dev-build"))]
pub const INSTANCE_MUTEX: PCWSTR = w!("Local\\Lilt.SingleInstance");

#[cfg(feature = "dev-build")]
pub const ISLAND_CLASS: PCWSTR = w!("Lilt.Dev.Island");
#[cfg(not(feature = "dev-build"))]
pub const ISLAND_CLASS: PCWSTR = w!("Lilt.Island");

#[cfg(feature = "dev-build")]
pub const WINDOW_CLASS: PCWSTR = w!("Lilt.Dev.Window");
#[cfg(not(feature = "dev-build"))]
pub const WINDOW_CLASS: PCWSTR = w!("Lilt.Window");

#[cfg(feature = "dev-build")]
pub const CONTROL_CLASS: PCWSTR = w!("Lilt.Dev.Control");
#[cfg(not(feature = "dev-build"))]
pub const CONTROL_CLASS: PCWSTR = w!("Lilt.Control");

#[cfg(feature = "dev-build")]
pub const STARTUP_VALUE: PCWSTR = w!("Lilt Dev");
#[cfg(not(feature = "dev-build"))]
pub const STARTUP_VALUE: PCWSTR = w!("Lilt");
