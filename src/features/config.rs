use core::cell::RefCell;

use critical_section::Mutex;
use trouble_host::Address;

pub(crate) const MAX_SCAN_RESULTS: usize = 12;

// Set these values before building to enable a station connection at boot.
// The credentials are embedded in the firmware image; leave the SSID empty to disable it.
pub(crate) const BOOT_WIFI_SSID: &str = "CP";
pub(crate) const BOOT_WIFI_PASSWORD: &str = "123456789";
pub(crate) const DEFAULT_UTC_OFFSET_HOURS: i8 = 8;
const MIN_UTC_OFFSET_HOURS: i8 = -12;
const MAX_UTC_OFFSET_HOURS: i8 = 14;

pub(crate) const WIFI_SCAN_IDLE: u8 = 0;
pub(crate) const WIFI_SCAN_REQUESTED: u8 = 1;
pub(crate) const WIFI_SCAN_RUNNING: u8 = 2;
pub(crate) const WIFI_SCAN_READY: u8 = 3;
pub(crate) const WIFI_SCAN_FAILED: u8 = 4;

pub(crate) const WIFI_CONNECTION_DISABLED: u8 = 0;
pub(crate) const WIFI_CONNECTION_DISCONNECTED: u8 = 1;
pub(crate) const WIFI_CONNECTION_CONNECTING: u8 = 2;
pub(crate) const WIFI_CONNECTION_CONNECTED: u8 = 3;
pub(crate) const WIFI_CONNECTION_FAILED: u8 = 4;

pub(crate) const BLE_SCAN_IDLE: u8 = 0;
pub(crate) const BLE_SCAN_REQUESTED: u8 = 1;
pub(crate) const BLE_SCAN_RUNNING: u8 = 2;
pub(crate) const BLE_SCAN_READY: u8 = 3;
pub(crate) const BLE_SCAN_FAILED: u8 = 4;

pub(crate) const BLE_PAIR_IDLE: u8 = 0;
pub(crate) const BLE_PAIR_REQUESTED: u8 = 1;
pub(crate) const BLE_PAIR_CONNECTING: u8 = 2;
pub(crate) const BLE_PAIR_WAITING_INPUT: u8 = 3;
pub(crate) const BLE_PAIR_PAIRED: u8 = 4;
pub(crate) const BLE_PAIR_FAILED: u8 = 5;
pub(crate) const BLE_PAIR_DISPLAY: u8 = 6;

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

    pub(crate) fn from_strings(ssid: &str, password: &str) -> Self {
        let mut credentials = Self::empty();
        credentials.ssid_len = copy_string(&mut credentials.ssid, ssid);
        credentials.password_len = copy_string(&mut credentials.password, password);
        credentials
    }
}

pub(crate) fn boot_wifi_credentials() -> Option<WifiCredentials> {
    if BOOT_WIFI_SSID.is_empty() {
        None
    } else {
        Some(WifiCredentials::from_strings(
            BOOT_WIFI_SSID,
            BOOT_WIFI_PASSWORD,
        ))
    }
}

#[derive(Clone, Copy)]
pub(crate) struct WifiControlCommand {
    pub(crate) ap_enabled: Option<bool>,
    pub(crate) station_enabled: Option<bool>,
    pub(crate) disconnect: bool,
}

impl WifiControlCommand {
    const fn empty() -> Self {
        Self {
            ap_enabled: None,
            station_enabled: None,
            disconnect: false,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct WifiStatusSnapshot {
    pub(crate) ap_enabled: bool,
    pub(crate) station_enabled: bool,
    pub(crate) connection_state: u8,
    pub(crate) connection_ssid: [u8; 32],
    pub(crate) connection_ssid_len: usize,
}

impl WifiStatusSnapshot {
    const fn initial() -> Self {
        Self {
            ap_enabled: false,
            station_enabled: true,
            connection_state: WIFI_CONNECTION_DISCONNECTED,
            connection_ssid: [0; 32],
            connection_ssid_len: 0,
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) struct WifiScanEntry {
    pub(crate) ssid: [u8; 32],
    pub(crate) ssid_len: usize,
    pub(crate) signal_strength: i8,
    pub(crate) secured: bool,
}

impl WifiScanEntry {
    pub(crate) const fn empty() -> Self {
        Self {
            ssid: [0; 32],
            ssid_len: 0,
            signal_strength: 0,
            secured: false,
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) struct WifiScanSnapshot {
    pub(crate) state: u8,
    pub(crate) count: usize,
    pub(crate) entries: [WifiScanEntry; MAX_SCAN_RESULTS],
}

impl WifiScanSnapshot {
    const fn empty() -> Self {
        Self {
            state: WIFI_SCAN_IDLE,
            count: 0,
            entries: [WifiScanEntry::empty(); MAX_SCAN_RESULTS],
        }
    }
}

static WIFI_COMMAND: Mutex<RefCell<Option<WifiCredentials>>> = Mutex::new(RefCell::new(None));
static WIFI_SCAN: Mutex<RefCell<WifiScanSnapshot>> =
    Mutex::new(RefCell::new(WifiScanSnapshot::empty()));
static WIFI_CONTROL: Mutex<RefCell<WifiControlCommand>> =
    Mutex::new(RefCell::new(WifiControlCommand::empty()));
static WIFI_STATUS_STATE: Mutex<RefCell<WifiStatusSnapshot>> =
    Mutex::new(RefCell::new(WifiStatusSnapshot::initial()));
static TIME_SYNC_TIMESTAMP: Mutex<RefCell<Option<u64>>> = Mutex::new(RefCell::new(None));
static UTC_OFFSET_HOURS: Mutex<RefCell<i8>> = Mutex::new(RefCell::new(DEFAULT_UTC_OFFSET_HOURS));

pub(crate) fn store_wifi_command(credentials: WifiCredentials) {
    crate::esp_info!(
        "WIFI: credentials queued, ssid_len={}, password_len={}",
        credentials.ssid_len,
        credentials.password_len
    );
    critical_section::with(|cs| *WIFI_COMMAND.borrow(cs).borrow_mut() = Some(credentials));
}

pub(crate) fn request_wifi_credentials(ssid: &str, password: &str) {
    store_wifi_command(WifiCredentials::from_strings(ssid, password));
}

pub(crate) fn take_wifi_command() -> Option<WifiCredentials> {
    critical_section::with(|cs| WIFI_COMMAND.borrow(cs).borrow_mut().take())
}

pub(crate) fn request_wifi_ap_state(enabled: bool) {
    crate::esp_info!("WIFI: AP state requested: enabled={}", enabled);
    critical_section::with(|cs| {
        WIFI_CONTROL.borrow(cs).borrow_mut().ap_enabled = Some(enabled);
    });
}

pub(crate) fn request_wifi_station_state(enabled: bool) {
    crate::esp_info!("WIFI: station state requested: enabled={}", enabled);
    critical_section::with(|cs| {
        WIFI_CONTROL.borrow(cs).borrow_mut().station_enabled = Some(enabled);
    });
}

pub(crate) fn request_wifi_disconnect() {
    crate::esp_info!("WIFI: disconnect requested");
    critical_section::with(|cs| {
        WIFI_CONTROL.borrow(cs).borrow_mut().disconnect = true;
    });
}

pub(crate) fn take_wifi_control() -> WifiControlCommand {
    critical_section::with(|cs| {
        let mut command = WIFI_CONTROL.borrow(cs).borrow_mut();
        let value = *command;
        *command = WifiControlCommand::empty();
        value
    })
}

pub(crate) fn set_wifi_mode_state(ap_enabled: bool, station_enabled: bool) {
    crate::esp_debug!(
        "WIFI: applying mode state, ap={}, station={}",
        ap_enabled,
        station_enabled
    );
    critical_section::with(|cs| {
        let mut status = WIFI_STATUS_STATE.borrow(cs).borrow_mut();
        status.ap_enabled = ap_enabled;
        status.station_enabled = station_enabled;
        if !station_enabled {
            status.connection_state = WIFI_CONNECTION_DISABLED;
            status.connection_ssid_len = 0;
        } else if status.connection_state == WIFI_CONNECTION_DISABLED {
            status.connection_state = WIFI_CONNECTION_DISCONNECTED;
        }
    });
}

pub(crate) fn set_wifi_connection(state: u8, ssid: Option<&str>) {
    crate::esp_debug!(
        "WIFI: updating connection state={}, ssid_present={}",
        state,
        ssid.is_some()
    );
    critical_section::with(|cs| {
        let mut status = WIFI_STATUS_STATE.borrow(cs).borrow_mut();
        status.connection_state = state;
        status.connection_ssid_len = 0;
        if let Some(ssid) = ssid {
            status.connection_ssid_len = copy_string(&mut status.connection_ssid, ssid);
        }
    });
}

pub(crate) fn copy_wifi_status() -> WifiStatusSnapshot {
    critical_section::with(|cs| *WIFI_STATUS_STATE.borrow(cs).borrow())
}

pub(crate) fn publish_time_sync(timestamp: u64) {
    crate::esp_info!("TIME: publishing network timestamp {}", timestamp);
    critical_section::with(|cs| {
        *TIME_SYNC_TIMESTAMP.borrow(cs).borrow_mut() = Some(timestamp);
    });
}

pub(crate) fn take_time_sync() -> Option<u64> {
    let timestamp = critical_section::with(|cs| TIME_SYNC_TIMESTAMP.borrow(cs).borrow_mut().take());
    if timestamp.is_some() {
        crate::esp_debug!("TIME: network timestamp consumed by UI");
    }
    timestamp
}

pub(crate) fn utc_offset_hours() -> i8 {
    critical_section::with(|cs| *UTC_OFFSET_HOURS.borrow(cs).borrow())
}

pub(crate) fn adjust_utc_offset(delta: i32) -> i8 {
    let current = utc_offset_hours();
    let next = (i32::from(current) + delta).clamp(
        i32::from(MIN_UTC_OFFSET_HOURS),
        i32::from(MAX_UTC_OFFSET_HOURS),
    ) as i8;
    if next != current {
        critical_section::with(|cs| {
            *UTC_OFFSET_HOURS.borrow(cs).borrow_mut() = next;
        });
        crate::esp_info!("TIME: UTC offset changed to UTC{:+}", next);
    }
    next
}

pub(crate) fn reset_utc_offset() -> i8 {
    critical_section::with(|cs| {
        *UTC_OFFSET_HOURS.borrow(cs).borrow_mut() = DEFAULT_UTC_OFFSET_HOURS;
    });
    crate::esp_info!(
        "TIME: UTC offset reset to UTC{:+}",
        DEFAULT_UTC_OFFSET_HOURS
    );
    DEFAULT_UTC_OFFSET_HOURS
}

pub(crate) fn request_wifi_scan() {
    crate::esp_info!("WIFI: scan request queued");
    critical_section::with(|cs| {
        let mut scan = WIFI_SCAN.borrow(cs).borrow_mut();
        if scan.state != WIFI_SCAN_RUNNING {
            scan.state = WIFI_SCAN_REQUESTED;
        }
    });
}

pub(crate) fn take_wifi_scan_request() -> bool {
    let requested = critical_section::with(|cs| {
        let mut scan = WIFI_SCAN.borrow(cs).borrow_mut();
        if scan.state == WIFI_SCAN_REQUESTED {
            scan.state = WIFI_SCAN_RUNNING;
            scan.count = 0;
            scan.entries = [WifiScanEntry::empty(); MAX_SCAN_RESULTS];
            true
        } else {
            false
        }
    });
    if requested {
        crate::esp_info!("WIFI: scan worker started");
    }
    requested
}

pub(crate) fn finish_wifi_scan(entries: [WifiScanEntry; MAX_SCAN_RESULTS], count: usize) {
    crate::esp_info!("WIFI: scan results ready, count={}", count);
    critical_section::with(|cs| {
        let mut scan = WIFI_SCAN.borrow(cs).borrow_mut();
        scan.entries = entries;
        scan.count = count.min(MAX_SCAN_RESULTS);
        scan.state = WIFI_SCAN_READY;
    });
}

pub(crate) fn fail_wifi_scan() {
    crate::esp_warn!("WIFI: scan state marked failed");
    critical_section::with(|cs| {
        let mut scan = WIFI_SCAN.borrow(cs).borrow_mut();
        scan.count = 0;
        scan.state = WIFI_SCAN_FAILED;
    });
}

pub(crate) fn copy_wifi_scan() -> WifiScanSnapshot {
    critical_section::with(|cs| *WIFI_SCAN.borrow(cs).borrow())
}

#[derive(Clone, Copy)]
pub(crate) struct BleScanEntry {
    pub(crate) name: [u8; 32],
    pub(crate) name_len: usize,
    pub(crate) address: Option<Address>,
    pub(crate) address_text: [u8; 17],
    pub(crate) address_len: usize,
    pub(crate) signal_strength: i8,
}

impl BleScanEntry {
    pub(crate) const fn empty() -> Self {
        Self {
            name: [0; 32],
            name_len: 0,
            address: None,
            address_text: [0; 17],
            address_len: 0,
            signal_strength: 0,
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) struct BleScanSnapshot {
    pub(crate) state: u8,
    pub(crate) count: usize,
    pub(crate) entries: [BleScanEntry; MAX_SCAN_RESULTS],
}

impl BleScanSnapshot {
    const fn empty() -> Self {
        Self {
            state: BLE_SCAN_IDLE,
            count: 0,
            entries: [BleScanEntry::empty(); MAX_SCAN_RESULTS],
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) struct BlePairRequest {
    pub(crate) address: Address,
    pub(crate) passkey: u32,
}

#[derive(Clone, Copy)]
pub(crate) struct BlePairSnapshot {
    pub(crate) state: u8,
    pub(crate) display_code: u32,
    pub(crate) confirm_requested: bool,
}

static BLE_SCAN: Mutex<RefCell<BleScanSnapshot>> =
    Mutex::new(RefCell::new(BleScanSnapshot::empty()));
static BLE_PAIR_COMMAND: Mutex<RefCell<Option<BlePairRequest>>> = Mutex::new(RefCell::new(None));
static BLE_PAIR_STATE: Mutex<RefCell<BlePairSnapshot>> =
    Mutex::new(RefCell::new(BlePairSnapshot {
        state: BLE_PAIR_IDLE,
        display_code: 0,
        confirm_requested: false,
    }));

pub(crate) fn request_ble_scan() {
    crate::esp_info!("BLE: scan request queued");
    critical_section::with(|cs| {
        let mut scan = BLE_SCAN.borrow(cs).borrow_mut();
        if scan.state != BLE_SCAN_RUNNING {
            scan.state = BLE_SCAN_REQUESTED;
        }
    });
}

pub(crate) fn take_ble_scan_request() -> bool {
    let requested = critical_section::with(|cs| {
        let mut scan = BLE_SCAN.borrow(cs).borrow_mut();
        if scan.state == BLE_SCAN_REQUESTED {
            scan.state = BLE_SCAN_RUNNING;
            scan.count = 0;
            scan.entries = [BleScanEntry::empty(); MAX_SCAN_RESULTS];
            true
        } else {
            false
        }
    });
    if requested {
        crate::esp_info!("BLE: scan worker started");
    }
    requested
}

pub(crate) fn store_ble_scan_entry(address: Address, name: &[u8], signal_strength: i8) {
    critical_section::with(|cs| {
        let mut scan = BLE_SCAN.borrow(cs).borrow_mut();
        let index = scan.entries[..scan.count]
            .iter()
            .position(|entry| entry.address == Some(address))
            .or_else(|| (scan.count < MAX_SCAN_RESULTS).then_some(scan.count));
        let Some(index) = index else {
            return;
        };

        let mut entry = BleScanEntry::empty();
        entry.address = Some(address);
        entry.signal_strength = signal_strength;
        entry.name_len = copy_bytes(&mut entry.name, name);
        if entry.name_len == 0 {
            entry.name_len = copy_address(&address, &mut entry.name);
        }
        entry.address_len = copy_address(&address, &mut entry.address_text);
        scan.entries[index] = entry;
        if index == scan.count {
            scan.count += 1;
        }
    });
}

pub(crate) fn finish_ble_scan() {
    crate::esp_info!("BLE: scan state marked ready");
    critical_section::with(|cs| {
        BLE_SCAN.borrow(cs).borrow_mut().state = BLE_SCAN_READY;
    });
}

pub(crate) fn fail_ble_scan() {
    crate::esp_warn!("BLE: scan state marked failed");
    critical_section::with(|cs| {
        let mut scan = BLE_SCAN.borrow(cs).borrow_mut();
        scan.count = 0;
        scan.state = BLE_SCAN_FAILED;
    });
}

pub(crate) fn copy_ble_scan() -> BleScanSnapshot {
    critical_section::with(|cs| *BLE_SCAN.borrow(cs).borrow())
}

pub(crate) fn request_ble_pairing(index: usize, passkey: u32) -> bool {
    crate::esp_info!("BLE: pairing validation requested for index {}", index);
    critical_section::with(|cs| {
        if !BLE_SETTINGS.borrow(cs).borrow().enabled {
            return false;
        }
        let scan = BLE_SCAN.borrow(cs).borrow();
        if index >= scan.count {
            return false;
        }
        let Some(address) = scan.entries[index].address else {
            return false;
        };
        *BLE_PAIR_COMMAND.borrow(cs).borrow_mut() = Some(BlePairRequest { address, passkey });
        let mut pairing = BLE_PAIR_STATE.borrow(cs).borrow_mut();
        pairing.state = BLE_PAIR_REQUESTED;
        pairing.display_code = 0;
        pairing.confirm_requested = false;
        true
    })
}

pub(crate) fn take_ble_pair_request() -> Option<BlePairRequest> {
    critical_section::with(|cs| BLE_PAIR_COMMAND.borrow(cs).borrow_mut().take())
}

pub(crate) fn set_ble_pair_state(state: u8, display_code: u32) {
    crate::esp_debug!("BLE: pairing state changed to {}", state);
    critical_section::with(|cs| {
        let mut pairing = BLE_PAIR_STATE.borrow(cs).borrow_mut();
        pairing.state = state;
        pairing.display_code = display_code;
    });
}

pub(crate) fn request_ble_pair_confirmation() {
    crate::esp_info!("BLE: pairing confirmation queued");
    critical_section::with(|cs| {
        BLE_PAIR_STATE.borrow(cs).borrow_mut().confirm_requested = true;
    });
}

pub(crate) fn take_ble_pair_confirmation() -> bool {
    critical_section::with(|cs| {
        let mut pairing = BLE_PAIR_STATE.borrow(cs).borrow_mut();
        let requested = pairing.confirm_requested;
        pairing.confirm_requested = false;
        requested
    })
}

pub(crate) fn copy_ble_pair_state() -> BlePairSnapshot {
    critical_section::with(|cs| *BLE_PAIR_STATE.borrow(cs).borrow())
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
        enabled: false,
    }
}

static BLE_SETTINGS: Mutex<RefCell<BleSettings>> = Mutex::new(RefCell::new(default_ble_settings()));

pub(crate) fn update_ble_name(name: [u8; 32], length: usize) {
    crate::esp_info!("BLE: advertising name updated, length={}", length);
    critical_section::with(|cs| {
        let mut settings = BLE_SETTINGS.borrow(cs).borrow_mut();
        settings.name = name;
        settings.name_len = length.min(settings.name.len());
    });
}

pub(crate) fn set_ble_enabled(enabled: bool) {
    crate::esp_info!("BLE: enabled={}", enabled);
    critical_section::with(|cs| BLE_SETTINGS.borrow(cs).borrow_mut().enabled = enabled);
}

pub(crate) fn copy_ble_enabled() -> bool {
    critical_section::with(|cs| BLE_SETTINGS.borrow(cs).borrow().enabled)
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

fn copy_string<const N: usize>(target: &mut [u8; N], value: &str) -> usize {
    copy_bytes(target, value.as_bytes())
}

fn copy_bytes<const N: usize>(target: &mut [u8; N], value: &[u8]) -> usize {
    let length = value.len().min(target.len());
    target[..length].copy_from_slice(&value[..length]);
    length
}

fn copy_address<const N: usize>(address: &Address, target: &mut [u8; N]) -> usize {
    let digits = b"0123456789ABCDEF";
    let raw = address.addr.raw();
    let mut output = 0;
    for (index, byte) in raw.iter().enumerate() {
        if output + 1 >= target.len() {
            break;
        }
        target[output] = digits[(byte >> 4) as usize];
        target[output + 1] = digits[(byte & 0x0f) as usize];
        output += 2;
        if index + 1 < raw.len() && output < target.len() {
            target[output] = b':';
            output += 1;
        }
    }
    output
}
