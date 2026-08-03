use crate::{
    drivers::{rtc::DateTime, touch::Cst816Touch},
    MainWindow,
};

fn digit(value: u16, divisor: u16) -> u8 {
    b'0' + ((value / divisor) % 10) as u8
}

pub fn update_clock(ui: &MainWindow, datetime: DateTime) {
    let mut clock = [b'0'; 8];
    clock[0] = digit(u16::from(datetime.hour), 10);
    clock[1] = digit(u16::from(datetime.hour), 1);
    clock[2] = b':';
    clock[3] = digit(u16::from(datetime.minute), 10);
    clock[4] = digit(u16::from(datetime.minute), 1);
    clock[5] = b':';
    clock[6] = digit(u16::from(datetime.second), 10);
    clock[7] = digit(u16::from(datetime.second), 1);

    let mut date = [b'0'; 10];
    date[0] = digit(datetime.year, 1000);
    date[1] = digit(datetime.year, 100);
    date[2] = digit(datetime.year, 10);
    date[3] = digit(datetime.year, 1);
    date[4] = b'-';
    date[5] = digit(u16::from(datetime.month), 10);
    date[6] = digit(u16::from(datetime.month), 1);
    date[7] = b'-';
    date[8] = digit(u16::from(datetime.day), 10);
    date[9] = digit(u16::from(datetime.day), 1);

    ui.set_clock_text(slint::SharedString::from(
        core::str::from_utf8(&clock).unwrap(),
    ));
    ui.set_date_text(slint::SharedString::from(
        core::str::from_utf8(&date).unwrap(),
    ));
}

pub fn set_rtc_unavailable(ui: &MainWindow) {
    crate::esp_warn!("RTC: clock UI marked unavailable");
    ui.set_clock_text(slint::SharedString::from("--:--:--"));
    ui.set_date_text(slint::SharedString::from("RTC 未设置"));
}

pub fn initialize_rtc(touch: &mut Cst816Touch<'_>) -> bool {
    let initialized = crate::drivers::rtc::init(touch).is_ok();
    crate::esp_info!("RTC: clock source initialized={}", initialized);
    initialized
}

pub fn refresh_rtc(ui: &MainWindow, touch: &mut Cst816Touch<'_>) {
    match crate::drivers::rtc::read_time(touch) {
        Ok(datetime) if datetime.is_valid() => update_clock(ui, datetime),
        _ => set_rtc_unavailable(ui),
    }
}

pub fn apply_network_time(ui: &MainWindow, touch: &mut Cst816Touch<'_>, timestamp: u64) -> bool {
    let Some(datetime) = DateTime::from_unix_seconds(timestamp) else {
        crate::esp_warn!(
            "RTC: network timestamp {} is outside supported range",
            timestamp
        );
        return false;
    };

    if crate::drivers::rtc::write_time(touch, datetime).is_err() {
        crate::esp_warn!("RTC: failed to apply network timestamp {}", timestamp);
        return false;
    }

    update_clock(ui, datetime);
    crate::esp_info!("RTC: network timestamp {} applied", timestamp);
    true
}
