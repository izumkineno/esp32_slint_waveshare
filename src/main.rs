#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those holding transfer buffers."
)]

extern crate alloc;

use alloc::string::String;
use core::fmt::Write as _;
use embassy_executor::Spawner;
use esp_hal::{clock::CpuClock, time::Instant};
use slint::{
    platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType, Rgb565Pixel},
    ComponentHandle, ModelRc, PhysicalSize, SharedString, VecModel,
};

use esp_slint_bsp::{board, drivers, features};
use esp_slint_bsp::{esp_debug, esp_info, esp_warn};

mod ui;
mod ui_logic;

slint::include_modules!();

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

// This creates the application descriptor required by the esp-idf bootloader.
esp_bootloader_esp_idf::esp_app_desc!();

#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    esp_println::logger::init_logger_from_env();
    crate::esp_info!("BOOT: esp_println logger initialized; direct monitor logging enabled");
    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);
    crate::esp_info!("BOOT: ESP32-S3 peripherals initialized");

    // 堆区(PSRAM 优先 + 内部 DRAM)在 board::init 内按正确顺序注册,
    // 以保证 WiFi/BLE 的 malloc_internal 与 DMA 独占内部 RAM。
    let (mut display, mut touch, _trng_source) = board::init(peripherals, spawner);
    crate::esp_info!("BOARD: drivers and background tasks initialized");
    let rtc_initialized = ui_logic::clock::initialize_rtc(&mut touch);
    crate::esp_info!(
        "RTC: initialization {}",
        if rtc_initialized {
            "succeeded"
        } else {
            "failed"
        }
    );

    let window = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    ui::install_platform(window.clone());
    crate::esp_info!("UI: Slint platform installed");

    let ui = MainWindow::new().unwrap();
    crate::esp_info!("UI: MainWindow created");
    ui.set_utc_offset_hours(i32::from(features::config::utc_offset_hours()));
    let clear_ui = ui.as_weak();
    ui.global::<AppState>().on_clear_requested(move || {
        if let Some(ui) = clear_ui.upgrade() {
            crate::esp_info!("UI: clear counter requested");
            ui.set_touch_count(0);
            ui.set_status_text(SharedString::from("就绪"));
        }
    });

    ui.global::<AppState>().on_wifi_scan_requested(|| {
        crate::esp_info!("WIFI: screen scan requested");
        features::config::request_wifi_scan();
    });

    ui.global::<AppState>()
        .on_wifi_ap_toggle_requested(|enabled| {
            crate::esp_info!("WIFI: AP enabled={}", enabled);
            features::config::request_wifi_ap_state(enabled);
        });

    ui.global::<AppState>()
        .on_wifi_station_toggle_requested(|enabled| {
            crate::esp_info!("WIFI: station enabled={}", enabled);
            features::config::request_wifi_station_state(enabled);
        });

    ui.global::<AppState>().on_wifi_disconnect_requested(|| {
        crate::esp_info!("WIFI: disconnect requested");
        features::config::request_wifi_disconnect();
    });

    let utc_offset_ui = ui.as_weak();
    ui.global::<AppState>()
        .on_utc_offset_adjust_requested(move |delta| {
            let offset = features::config::adjust_utc_offset(delta);
            if let Some(ui) = utc_offset_ui.upgrade() {
                ui.set_utc_offset_hours(i32::from(offset));
            }
        });

    let utc_offset_reset_ui = ui.as_weak();
    ui.global::<AppState>()
        .on_utc_offset_reset_requested(move || {
            let offset = features::config::reset_utc_offset();
            if let Some(ui) = utc_offset_reset_ui.upgrade() {
                ui.set_utc_offset_hours(i32::from(offset));
            }
        });

    let wifi_key_ui = ui.as_weak();
    ui.global::<AppState>().on_wifi_key_pressed(move |key| {
        if let Some(ui) = wifi_key_ui.upgrade() {
            if key == "SHIFT" {
                ui.set_wifi_shift(!ui.get_wifi_shift());
                return;
            }
            if key == "SYM" {
                ui.set_wifi_symbols(!ui.get_wifi_symbols());
                return;
            }

            let mut password = String::from(ui.get_wifi_password().as_str());
            if key == "BACK" {
                password.pop();
            } else if key == "SPACE" {
                if password.len() < 64 {
                    password.push(' ');
                }
            } else if let Some(mut byte) = key.as_bytes().first().copied() {
                if ui.get_wifi_shift() && (b'a'..=b'z').contains(&byte) {
                    byte -= b'a' - b'A';
                }
                if password.len() < 64 {
                    password.push(byte as char);
                }
            }

            ui.set_wifi_password(SharedString::from(password.as_str()));
        }
    });

    let wifi_submit_ui = ui.as_weak();
    ui.global::<AppState>().on_wifi_password_submit(move || {
        if let Some(ui) = wifi_submit_ui.upgrade() {
            crate::esp_info!(
                "WIFI: credentials submitted, ssid={}, password_len={}",
                ui.get_wifi_selected_ssid().as_str(),
                ui.get_wifi_password().len()
            );
            features::config::request_wifi_credentials(
                ui.get_wifi_selected_ssid().as_str(),
                ui.get_wifi_password().as_str(),
            );
            ui.set_status_text(SharedString::from("WiFi 凭据已提交，正在连接"));
            ui.set_menu_view(4);
        }
    });

    ui.global::<AppState>().on_ble_scan_requested(|| {
        crate::esp_info!("BLE: screen scan requested");
        features::config::request_ble_scan();
    });

    ui.global::<AppState>().on_ble_toggle_requested(|enabled| {
        crate::esp_info!("BLE: enabled={}", enabled);
        features::config::set_ble_enabled(enabled);
    });

    let ble_key_ui = ui.as_weak();
    ui.global::<AppState>().on_ble_key_pressed(move |key| {
        if let Some(ui) = ble_key_ui.upgrade() {
            let mut code = String::from(ui.get_ble_code().as_str());
            if key == "BACK" {
                code.pop();
            } else if code.len() < 6 && key.as_bytes().first().is_some_and(u8::is_ascii_digit) {
                code.push(key.as_bytes()[0] as char);
            }
            ui.set_ble_code(SharedString::from(code.as_str()));
        }
    });

    let ble_submit_ui = ui.as_weak();
    ui.global::<AppState>().on_ble_pair_submit(move || {
        if let Some(ui) = ble_submit_ui.upgrade() {
            let code_text = ui.get_ble_code();
            if ui.get_ble_selected_index() < 0 || code_text.len() != 6 {
                crate::esp_warn!("BLE: pairing rejected before request validation");
                ui.set_ble_pair_status(SharedString::from("请输入 6 位配对码"));
                return;
            }
            match code_text.as_str().parse::<u32>() {
                Ok(code)
                    if features::config::request_ble_pairing(
                        ui.get_ble_selected_index() as usize,
                        code,
                    ) =>
                {
                    ui.set_ble_pair_status(SharedString::from("正在连接并请求配对"));
                    crate::esp_info!("BLE: pairing request submitted");
                }
                _ => {
                    crate::esp_warn!("BLE: pairing request rejected because the list changed");
                    ui.set_ble_pair_status(SharedString::from("设备列表已更新，请重新选择"));
                }
            }
        }
    });

    ui.global::<AppState>().on_ble_pair_confirmed(|| {
        crate::esp_info!("BLE: pairing confirmation requested");
        features::config::request_ble_pair_confirmation();
    });

    if rtc_initialized {
        ui_logic::clock::refresh_rtc(&ui, &mut touch);
    } else {
        ui_logic::clock::set_rtc_unavailable(&ui);
    }
    crate::esp_info!("UI: initial clock state prepared");

    window.set_size(PhysicalSize::new(
        drivers::display::LCD_WIDTH as u32,
        drivers::display::LCD_HEIGHT as u32,
    ));
    window.request_redraw();
    crate::esp_info!(
        "UI: window configured at {}x{}",
        drivers::display::LCD_WIDTH,
        drivers::display::LCD_HEIGHT
    );

    let mut line_buffer = [Rgb565Pixel(0); drivers::display::LCD_WIDTH];
    let mut last_touch = None;
    let mut touch_start = None;
    let mut rtc_window_start = Instant::now();
    let mut fps_window_start = Instant::now();
    let mut heap_window_start = Instant::now();
    let mut rendered_frames: u32 = 0;
    let mut last_wifi_scan_state = u8::MAX;
    let mut last_wifi_scan_count = usize::MAX;
    let mut last_ble_scan_state = u8::MAX;
    let mut last_ble_scan_count = usize::MAX;
    let mut last_ble_pair_state = u8::MAX;
    let mut last_ble_pair_code = u32::MAX;
    let mut last_wifi_status = features::config::copy_wifi_status();
    let mut last_ble_enabled = features::config::copy_ble_enabled();
    let mut last_utc_offset = features::config::utc_offset_hours();

    loop {
        let wifi_status = features::config::copy_wifi_status();
        if wifi_status != last_wifi_status {
            update_wifi_status_ui(&ui, wifi_status);
            last_wifi_status = wifi_status;
        }

        let ble_enabled = features::config::copy_ble_enabled();
        if ble_enabled != last_ble_enabled {
            ui.set_ble_enabled(ble_enabled);
            last_ble_enabled = ble_enabled;
            crate::esp_info!("UI: BLE enabled state changed to {}", ble_enabled);
        }
        let wifi_scan = features::config::copy_wifi_scan();
        if wifi_scan.state != last_wifi_scan_state || wifi_scan.count != last_wifi_scan_count {
            update_wifi_scan_ui(&ui, wifi_scan);
            last_wifi_scan_state = wifi_scan.state;
            last_wifi_scan_count = wifi_scan.count;
        }

        let ble_scan = features::config::copy_ble_scan();
        if ble_scan.state != last_ble_scan_state || ble_scan.count != last_ble_scan_count {
            update_ble_scan_ui(&ui, ble_scan);
            last_ble_scan_state = ble_scan.state;
            last_ble_scan_count = ble_scan.count;
        }

        let ble_pair = features::config::copy_ble_pair_state();
        if ble_pair.state != last_ble_pair_state || ble_pair.display_code != last_ble_pair_code {
            update_ble_pair_ui(&ui, ble_pair);
            last_ble_pair_state = ble_pair.state;
            last_ble_pair_code = ble_pair.display_code;
        }

        let utc_offset = features::config::utc_offset_hours();
        if utc_offset != last_utc_offset {
            crate::esp_info!(
                "TIME: refreshing clock after UTC offset change to UTC{:+}",
                utc_offset
            );
            ui.set_utc_offset_hours(i32::from(utc_offset));
            if rtc_initialized {
                ui_logic::clock::refresh_rtc(&ui, &mut touch);
            }
            last_utc_offset = utc_offset;
        }

        if rtc_initialized {
            if let Some(timestamp) = features::config::take_time_sync() {
                let applied = ui_logic::clock::apply_network_time(&ui, &mut touch, timestamp);
                crate::esp_info!(
                    "TIME: network timestamp {} applied to RTC: {}",
                    timestamp,
                    applied
                );
            }
        }
        slint::platform::update_timers_and_animations();

        if let Some(swipe) =
            ui_logic::input::poll_touch(&window, &mut touch, &mut last_touch, &mut touch_start)
        {
            crate::esp_debug!("UI: horizontal swipe detected");
            match (swipe, ui.get_menu_open()) {
                (ui_logic::SwipeDirection::Right, false) => {
                    ui.set_menu_view(0);
                    ui.set_menu_open(true);
                    crate::esp_info!("UI: opened feature menu");
                }
                (ui_logic::SwipeDirection::Left, true) => {
                    ui.set_menu_view(0);
                    ui.set_menu_open(false);
                    crate::esp_info!("UI: returned to clock home");
                }
                _ => {}
            }
        }

        if rtc_initialized && rtc_window_start.elapsed().as_millis() >= 1_000 {
            ui_logic::clock::refresh_rtc(&ui, &mut touch);
            rtc_window_start = Instant::now();
        }

        if window.draw_if_needed(|renderer| {
            let display_buffer = ui::DisplayLineBuffer {
                display: &mut display,
                buffer: &mut line_buffer,
            };
            renderer.render_by_line(display_buffer);
        }) {
            rendered_frames += 1;
        }

        let elapsed_ms = fps_window_start.elapsed().as_millis();
        if elapsed_ms >= 1_000 {
            let fps = (u64::from(rendered_frames) * 1_000 / elapsed_ms) as i32;
            ui.set_fps(fps);
            rendered_frames = 0;
            // crate::esp_debug!("UI: render fps={}", fps);
            fps_window_start = Instant::now();
        }

        // 每 10 秒打印内部/外部堆余量,用于确认 WiFi 独占的内部 RAM 未被榨干。
        // 若 free_internal 长期接近 0,WiFi 发包会再次出现 ESP_ERR_NO_MEM(257)。
        if heap_window_start.elapsed().as_millis() >= 10_000 {
            crate::esp_info!(
                "MEM: free_internal={} bytes, free_external={} bytes",
                esp_alloc::HEAP.free_caps(esp_alloc::MemoryCapability::Internal.into()),
                esp_alloc::HEAP.free_caps(esp_alloc::MemoryCapability::External.into())
            );
            heap_window_start = Instant::now();
        }

        embassy_time::Timer::after_millis(10).await;
    }
}

fn update_wifi_status_ui(ui: &MainWindow, status: features::config::WifiStatusSnapshot) {
    crate::esp_debug!(
        "UI: WiFi status changed, ap={}, station={}, connection={}",
        status.ap_enabled,
        status.station_enabled,
        status.connection_state
    );
    ui.set_wifi_ap_enabled(status.ap_enabled);
    ui.set_wifi_ap_status(SharedString::from(if status.ap_enabled {
        "已开启"
    } else {
        "已关闭"
    }));
    ui.set_wifi_station_enabled(status.station_enabled);

    let connection_status = match status.connection_state {
        features::config::WIFI_CONNECTION_DISABLED => "已关闭",
        features::config::WIFI_CONNECTION_CONNECTING => "连接中",
        features::config::WIFI_CONNECTION_CONNECTED => "已连接",
        features::config::WIFI_CONNECTION_FAILED => "连接失败",
        _ => "未连接",
    };
    ui.set_wifi_connection_status(SharedString::from(connection_status));

    if status.connection_ssid_len > 0 {
        ui.set_wifi_connection_ssid(bytes_to_shared(
            &status.connection_ssid,
            status.connection_ssid_len,
        ));
    } else {
        ui.set_wifi_connection_ssid(SharedString::from("无目标网络"));
    }
}

fn update_wifi_scan_ui(ui: &MainWindow, snapshot: features::config::WifiScanSnapshot) {
    crate::esp_debug!(
        "UI: WiFi scan state={}, count={}",
        snapshot.state,
        snapshot.count
    );
    ui.set_wifi_scan_state(snapshot.state as i32);
    ui.set_wifi_network_count(snapshot.count as i32);

    let mut status = String::new();
    match snapshot.state {
        features::config::WIFI_SCAN_REQUESTED | features::config::WIFI_SCAN_RUNNING => {
            status.push_str("正在扫描附近网络…");
        }
        features::config::WIFI_SCAN_READY => {
            let _ = write!(
                &mut status,
                "扫描完成：{} 个网络，向上滑动查看更多",
                snapshot.count
            );
        }
        features::config::WIFI_SCAN_FAILED => status.push_str("扫描失败，请重试"),
        _ => status.push_str("点击扫描附近网络"),
    }
    ui.set_wifi_scan_status(SharedString::from(status.as_str()));

    let names = ModelRc::new(VecModel::from_iter(
        snapshot
            .entries
            .iter()
            .take(snapshot.count)
            .map(|entry| bytes_to_shared(&entry.ssid, entry.ssid_len)),
    ));
    let details = ModelRc::new(VecModel::from_iter(
        snapshot
            .entries
            .iter()
            .take(snapshot.count)
            .map(wifi_detail),
    ));
    ui.set_wifi_networks(names);
    ui.set_wifi_network_details(details);
}

fn update_ble_scan_ui(ui: &MainWindow, snapshot: features::config::BleScanSnapshot) {
    crate::esp_debug!(
        "UI: BLE scan state={}, count={}",
        snapshot.state,
        snapshot.count
    );
    ui.set_ble_scan_state(snapshot.state as i32);
    ui.set_ble_device_count(snapshot.count as i32);

    let mut status = String::new();
    match snapshot.state {
        features::config::BLE_SCAN_REQUESTED | features::config::BLE_SCAN_RUNNING => {
            status.push_str("正在扫描附近 BLE 设备…");
        }
        features::config::BLE_SCAN_READY => {
            let _ = write!(&mut status, "扫描完成：{} 个设备", snapshot.count);
        }
        features::config::BLE_SCAN_FAILED => status.push_str("扫描失败，请重试"),
        _ => status.push_str("点击扫描附近设备"),
    }
    ui.set_ble_scan_status(SharedString::from(status.as_str()));

    let names = ModelRc::new(VecModel::from_iter(
        snapshot.entries.iter().take(snapshot.count).map(ble_name),
    ));
    let details = ModelRc::new(VecModel::from_iter(
        snapshot.entries.iter().take(snapshot.count).map(ble_detail),
    ));
    ui.set_ble_devices(names);
    ui.set_ble_device_details(details);
}

fn update_ble_pair_ui(ui: &MainWindow, snapshot: features::config::BlePairSnapshot) {
    crate::esp_debug!(
        "UI: BLE pairing state={}, display_code_present={}",
        snapshot.state,
        snapshot.display_code != 0
    );
    ui.set_ble_pair_state(snapshot.state as i32);
    let mut display_code = String::new();
    if snapshot.display_code != 0 {
        let _ = write!(&mut display_code, "{:06}", snapshot.display_code);
        ui.set_ble_code(SharedString::from(display_code.as_str()));
    }
    ui.set_ble_display_code(SharedString::from(display_code.as_str()));

    let status = match snapshot.state {
        features::config::BLE_PAIR_REQUESTED => "正在准备配对",
        features::config::BLE_PAIR_CONNECTING => "正在连接设备…",
        features::config::BLE_PAIR_WAITING_INPUT => "正在提交配对码…",
        features::config::BLE_PAIR_PAIRED => "配对成功",
        features::config::BLE_PAIR_FAILED => "配对失败，请重新扫描",
        features::config::BLE_PAIR_DISPLAY => "请核对设备显示的配对码并确认",
        _ => "请输入 6 位配对码",
    };
    ui.set_ble_pair_status(SharedString::from(status));
}

fn bytes_to_shared(bytes: &[u8; 32], length: usize) -> SharedString {
    match core::str::from_utf8(&bytes[..length.min(bytes.len())]) {
        Ok(value) if !value.is_empty() => SharedString::from(value),
        _ => SharedString::from("不可显示的网络"),
    }
}

fn wifi_detail(entry: &features::config::WifiScanEntry) -> SharedString {
    let mut detail = String::new();
    let security = if entry.secured { "加密" } else { "开放" };
    let _ = write!(&mut detail, "{} dBm · {}", entry.signal_strength, security);
    SharedString::from(detail.as_str())
}

fn ble_name(entry: &features::config::BleScanEntry) -> SharedString {
    if entry.name_len > 0 {
        if let Ok(value) = core::str::from_utf8(&entry.name[..entry.name_len.min(entry.name.len())])
        {
            return SharedString::from(value);
        }
    }
    let length = entry.address_len.min(entry.address_text.len());
    let mut address = [0u8; 32];
    address[..length].copy_from_slice(&entry.address_text[..length]);
    bytes_to_shared(&address, length)
}

fn ble_detail(entry: &features::config::BleScanEntry) -> SharedString {
    let mut detail = String::new();
    if let Ok(address) =
        core::str::from_utf8(&entry.address_text[..entry.address_len.min(entry.address_text.len())])
    {
        let _ = write!(&mut detail, "{} · {} dBm", address, entry.signal_strength);
    } else {
        let _ = write!(&mut detail, "{} dBm", entry.signal_strength);
    }
    SharedString::from(detail.as_str())
}
