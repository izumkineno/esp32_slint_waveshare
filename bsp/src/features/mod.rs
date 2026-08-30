#[cfg(any(feature = "wifi", feature = "ble"))]
pub mod config;

#[cfg(feature = "ble")]
pub mod bluetooth;
#[cfg(feature = "wifi")]
pub mod time_sync;
#[cfg(feature = "wifi")]
pub mod wifi_portal;
