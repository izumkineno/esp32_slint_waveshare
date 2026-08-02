use core::cell::RefCell;

use critical_section::Mutex;

#[derive(Clone, Copy)]
pub(crate) struct WifiCredentials {
    pub(crate) ssid: [u8; 32],
    pub(crate) ssid_len: usize,
    pub(crate) password: [u8; 64],
    pub(crate) password_len: usize,
}

impl WifiCredentials {
    pub(crate) const fn empty() -> Self {
        Self {
            ssid: [0; 32],
            ssid_len: 0,
            password: [0; 64],
            password_len: 0,
        }
    }
}

#[derive(Clone, Copy)]
struct BleSettings {
    name: [u8; 32],
    name_len: usize,
    enabled: bool,
}

const fn default_ble_settings() -> BleSettings {
    let source = b"ESP32-S3-BLE";
    let mut name = [0; 32];
    let mut index = 0;
    while index < source.len() {
        name[index] = source[index];
        index += 1;
    }
    BleSettings {
        name,
        name_len: source.len(),
        enabled: true,
    }
}

static WIFI_COMMAND: Mutex<RefCell<Option<WifiCredentials>>> = Mutex::new(RefCell::new(None));
static BLE_SETTINGS: Mutex<RefCell<BleSettings>> = Mutex::new(RefCell::new(default_ble_settings()));

pub(crate) fn store_wifi_command(credentials: WifiCredentials) {
    critical_section::with(|cs| *WIFI_COMMAND.borrow(cs).borrow_mut() = Some(credentials));
}

pub(crate) fn take_wifi_command() -> Option<WifiCredentials> {
    critical_section::with(|cs| WIFI_COMMAND.borrow(cs).borrow_mut().take())
}

pub(crate) fn update_ble_name(name: [u8; 32], length: usize) {
    critical_section::with(|cs| {
        let mut settings = BLE_SETTINGS.borrow(cs).borrow_mut();
        settings.name = name;
        settings.name_len = length.min(settings.name.len());
    });
}

pub(crate) fn set_ble_enabled(enabled: bool) {
    critical_section::with(|cs| BLE_SETTINGS.borrow(cs).borrow_mut().enabled = enabled);
}

pub(crate) fn copy_ble_name(buffer: &mut [u8; 32]) -> (usize, bool) {
    critical_section::with(|cs| {
        let settings = BLE_SETTINGS.borrow(cs).borrow();
        buffer.fill(0);
        let length = settings.name_len.min(buffer.len());
        buffer[..length].copy_from_slice(&settings.name[..length]);
        (length, settings.enabled)
    })
}
