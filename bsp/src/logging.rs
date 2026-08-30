//! Direct ESP monitor logging helpers.
//!
//! Application logs go through `esp_println::println!` so they are emitted on
//! the same UART or USB JTAG stream that `espflash monitor` reads. Unlike the
//! optional `log` facade, these messages are not filtered by `ESP_LOG`.

#[macro_export]
macro_rules! esp_log {
    ($level:literal, $($arg:tt)*) => {{
        ::esp_println::println!(
            "[{}][{}] {}",
            $level,
            module_path!(),
            ::core::format_args!($($arg)*)
        );
    }};
}

#[macro_export]
macro_rules! esp_trace {
    ($($arg:tt)*) => {
        $crate::esp_log!("TRACE", $($arg)*)
    };
}

#[macro_export]
macro_rules! esp_debug {
    ($($arg:tt)*) => {
        $crate::esp_log!("DEBUG", $($arg)*)
    };
}

#[macro_export]
macro_rules! esp_info {
    ($($arg:tt)*) => {
        $crate::esp_log!("INFO", $($arg)*)
    };
}

#[macro_export]
macro_rules! esp_warn {
    ($($arg:tt)*) => {
        $crate::esp_log!("WARN", $($arg)*)
    };
}

#[macro_export]
macro_rules! esp_error {
    ($($arg:tt)*) => {
        $crate::esp_log!("ERROR", $($arg)*)
    };
}
