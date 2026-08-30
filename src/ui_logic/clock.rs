use crate::MainWindow;
use esp_slint_bsp::{
    drivers::{rtc::DateTime, touch::Cst816Touch},
    features::config,
};
fn digit(value: u16, divisor: u16) -> u8 {
    b'0' + ((value / divisor) % 10) as u8
}

const WEEKDAY_TEXTS: [&str; 7] = [
    "(周日)", "(周一)", "(周二)", "(周三)", "(周四)", "(周五)", "(周六)",
];

pub fn update_clock(ui: &MainWindow, datetime: DateTime) {
    let mut clock = [b'0'; 5];
    clock[0] = digit(u16::from(datetime.hour), 10);
    clock[1] = digit(u16::from(datetime.hour), 1);
    clock[2] = b':';
    clock[3] = digit(u16::from(datetime.minute), 10);
    clock[4] = digit(u16::from(datetime.minute), 1);

    let mut seconds = [b'0'; 2];
    seconds[0] = digit(u16::from(datetime.second), 10);
    seconds[1] = digit(u16::from(datetime.second), 1);

    let mut date = [0u8; 5];
    date[0] = digit(u16::from(datetime.day), 10);
    date[1] = digit(u16::from(datetime.day), 1);
    date[2..].copy_from_slice("日".as_bytes());

    ui.set_clock_text(slint::SharedString::from(
        core::str::from_utf8(&clock).unwrap(),
    ));
    ui.set_seconds_text(slint::SharedString::from(
        core::str::from_utf8(&seconds).unwrap(),
    ));
    ui.set_seconds_value(i32::from(datetime.second));
    ui.set_date_text(slint::SharedString::from(
        core::str::from_utf8(&date).unwrap(),
    ));
    ui.set_weekday_text(slint::SharedString::from(
        WEEKDAY_TEXTS[usize::from(datetime.weekday.min(6))],
    ));
}

pub fn set_rtc_unavailable(ui: &MainWindow) {
    crate::esp_warn!("RTC: clock UI marked unavailable");
    ui.set_clock_text(slint::SharedString::from("--:--"));
    ui.set_seconds_text(slint::SharedString::from("--"));
    ui.set_seconds_value(-1);
    ui.set_date_text(slint::SharedString::from("--日"));
    ui.set_weekday_text(slint::SharedString::from("(--)"));
}

pub fn initialize_rtc(touch: &mut Cst816Touch<'_>) -> bool {
    let initialized = esp_slint_bsp::drivers::rtc::init(touch).is_ok();
    crate::esp_info!("RTC: clock source initialized={}", initialized);
    initialized
}

pub fn refresh_rtc(ui: &MainWindow, touch: &mut Cst816Touch<'_>) {
    match esp_slint_bsp::drivers::rtc::read_time(touch) {
        Ok(utc_datetime) if utc_datetime.is_valid() => {
            let local_datetime = utc_datetime
                .with_utc_offset(config::utc_offset_hours())
                .unwrap_or(utc_datetime);
            update_clock(ui, local_datetime);
        }
        _ => set_rtc_unavailable(ui),
    }
}

pub fn apply_network_time(ui: &MainWindow, touch: &mut Cst816Touch<'_>, timestamp: u64) -> bool {
    let Some(utc_datetime) = DateTime::from_unix_seconds(timestamp) else {
        crate::esp_warn!(
            "RTC: network timestamp {} is outside supported range",
            timestamp
        );
        return false;
    };

    if esp_slint_bsp::drivers::rtc::write_time(touch, utc_datetime).is_err() {
        crate::esp_warn!("RTC: failed to write network timestamp {}", timestamp);
        return false;
    }

    let local_datetime = utc_datetime
        .with_utc_offset(config::utc_offset_hours())
        .unwrap_or(utc_datetime);
    update_clock(ui, local_datetime);
    crate::esp_info!(
        "RTC: network timestamp {} applied with UTC offset {:+}",
        timestamp,
        config::utc_offset_hours()
    );
    true
}
