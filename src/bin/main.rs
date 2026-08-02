#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those holding transfer buffers."
)]

extern crate alloc;

use embassy_executor::Spawner;
use esp_hal::{clock::CpuClock, delay::Delay, ram, time::Instant};
use slint::{
    platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType, Rgb565Pixel},
    ComponentHandle, PhysicalSize,
};

#[path = "../board/mod.rs"]
mod board;
#[path = "../drivers/mod.rs"]
mod drivers;
#[path = "../features/mod.rs"]
mod features;
#[path = "../ui/mod.rs"]
mod ui;
#[path = "../ui_logic/mod.rs"]
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
    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    esp_alloc::heap_allocator!(#[ram(reclaimed)] size: 64 * 1024);
    esp_alloc::heap_allocator!(size: 36 * 1024);

    let (mut display, mut touch) = board::init(peripherals, spawner);
    let rtc_initialized = ui_logic::clock::initialize_rtc(&mut touch);

    let window = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    ui::install_platform(window.clone());

    let ui = MainWindow::new().unwrap();
    let ui_weak = ui.as_weak();
    ui.on_clear_requested(move || {
        if let Some(ui) = ui_weak.upgrade() {
            ui.set_touch_count(0);
            ui.set_status_text(slint::SharedString::from("就绪"));
        }
    });

    if rtc_initialized {
        ui_logic::clock::refresh_rtc(&ui, &mut touch);
    } else {
        ui_logic::clock::set_rtc_unavailable(&ui);
    }

    window.set_size(PhysicalSize::new(
        drivers::display::LCD_WIDTH as u32,
        drivers::display::LCD_HEIGHT as u32,
    ));
    window.request_redraw();

    let delay = Delay::new();
    let mut line_buffer = [Rgb565Pixel(0); drivers::display::LCD_WIDTH];
    let mut last_touch = None;
    let mut touch_start = None;
    let mut rtc_window_start = Instant::now();
    let mut fps_window_start = Instant::now();
    let mut rendered_frames: u32 = 0;

    loop {
        slint::platform::update_timers_and_animations();

        if let Some(swipe) =
            ui_logic::input::poll_touch(&window, &mut touch, &mut last_touch, &mut touch_start)
        {
            match (swipe, ui.get_menu_open()) {
                (ui_logic::SwipeDirection::Right, false) => {
                    ui.set_menu_view(0);
                    ui.set_menu_open(true);
                }
                (ui_logic::SwipeDirection::Left, true) => {
                    ui.set_menu_view(0);
                    ui.set_menu_open(false);
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
            fps_window_start = Instant::now();
        }

        delay.delay_millis(10);
    }
}
