#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those holding transfer buffers."
)]

extern crate alloc;

use alloc::{boxed::Box, rc::Rc};
use core::ops::Range;

use esp_hal::{clock::CpuClock, delay::Delay, main, time::Instant};
use slint::{
    platform::{
        software_renderer::{
            LineBufferProvider, MinimalSoftwareWindow, RepaintBufferType, Rgb565Pixel,
        },
        Platform, WindowEvent,
    },
    ComponentHandle, LogicalPosition, PhysicalPosition, PhysicalSize,
};

#[path = "../../../esp_learn/src/cst816.rs"]
mod cst816;
#[path = "../st77916.rs"]
mod st77916;

slint::include_modules!();

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

// This creates the application descriptor required by the esp-idf bootloader.
esp_bootloader_esp_idf::esp_app_desc!();

struct EspPlatform {
    window: Rc<MinimalSoftwareWindow>,
}

impl Platform for EspPlatform {
    fn create_window_adapter(
        &self,
    ) -> Result<Rc<dyn slint::platform::WindowAdapter>, slint::PlatformError> {
        Ok(self.window.clone())
    }

    fn duration_since_start(&self) -> core::time::Duration {
        core::time::Duration::from_millis(Instant::now().duration_since_epoch().as_millis())
    }
}

struct DisplayLineBuffer<'a, 'd> {
    display: &'a mut st77916::St77916Display<'d>,
    buffer: &'a mut [Rgb565Pixel],
}

impl<'a, 'd> LineBufferProvider for DisplayLineBuffer<'a, 'd> {
    type TargetPixel = Rgb565Pixel;

    fn process_line(
        &mut self,
        line: usize,
        range: Range<usize>,
        render_fn: impl FnOnce(&mut [Self::TargetPixel]),
    ) {
        let pixels = &mut self.buffer[range.clone()];
        render_fn(pixels);
        self.display.write_line(line, range, pixels).unwrap();
    }
}

fn poll_touch(
    window: &MinimalSoftwareWindow,
    touch: &mut cst816::Cst816Touch<'_>,
    last_touch: &mut Option<LogicalPosition>,
) {
    match touch.read() {
        Ok(Some(point)) => {
            let position = PhysicalPosition::new(point.x as i32, point.y as i32)
                .to_logical(window.scale_factor());

            if let Some(previous) = last_touch.replace(position) {
                if previous != position {
                    window.dispatch_event(WindowEvent::PointerMoved { position });
                }
            } else {
                window.dispatch_event(WindowEvent::PointerPressed {
                    position,
                    button: slint::platform::PointerEventButton::Left,
                });
            }
        }
        Ok(None) | Err(_) => {
            // A missing or failed I2C sample ends the current contact.
            if let Some(position) = last_touch.take() {
                window.dispatch_event(WindowEvent::PointerReleased {
                    position,
                    button: slint::platform::PointerEventButton::Left,
                });
                window.dispatch_event(WindowEvent::PointerExited);
            }
        }
    }
}

#[main]
fn main() -> ! {
    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    let (mut display, mut touch) = st77916::init(peripherals);

    let window = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    slint::platform::set_platform(Box::new(EspPlatform {
        window: window.clone(),
    }))
    .unwrap();

    let ui = MainWindow::new().unwrap();
    let ui_weak = ui.as_weak();
    ui.on_clear_requested(move || {
        if let Some(ui) = ui_weak.upgrade() {
            ui.set_touch_count(0);
            ui.set_status_text(slint::SharedString::from("Ready"));
        }
    });

    window.set_size(PhysicalSize::new(
        st77916::LCD_WIDTH as u32,
        st77916::LCD_HEIGHT as u32,
    ));
    window.request_redraw();

    let delay = Delay::new();
    let mut line_buffer = [Rgb565Pixel(0); st77916::LCD_WIDTH];
    let mut last_touch = None;
    let mut fps_window_start = Instant::now();
    let mut rendered_frames: u32 = 0;

    loop {
        slint::platform::update_timers_and_animations();
        poll_touch(&window, &mut touch, &mut last_touch);

        if window.draw_if_needed(|renderer| {
            let display_buffer = DisplayLineBuffer {
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
