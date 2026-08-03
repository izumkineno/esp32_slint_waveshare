use alloc::{boxed::Box, rc::Rc};
use core::ops::Range;

use esp_hal::time::Instant;
use slint::platform::{
    software_renderer::{LineBufferProvider, MinimalSoftwareWindow, Rgb565Pixel},
    Platform,
};

use crate::drivers::display::St77916Display;

pub struct EspPlatform {
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

pub struct DisplayLineBuffer<'a, 'd> {
    pub display: &'a mut St77916Display<'d>,
    pub buffer: &'a mut [Rgb565Pixel],
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

pub fn install_platform(window: Rc<MinimalSoftwareWindow>) {
    crate::esp_info!("UI: installing ESP Slint platform");
    slint::platform::set_platform(Box::new(EspPlatform { window })).unwrap();
    crate::esp_info!("UI: ESP Slint platform installed");
}
