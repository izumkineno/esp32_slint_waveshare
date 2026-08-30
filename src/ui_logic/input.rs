use esp_slint_bsp::drivers::touch::Cst816Touch;
use slint::{
    platform::{software_renderer::MinimalSoftwareWindow, PointerEventButton, WindowEvent},
    LogicalPosition, PhysicalPosition,
};

#[derive(Clone, Copy, Eq, PartialEq)]
pub enum SwipeDirection {
    Right,
    Left,
}

pub fn poll_touch(
    window: &MinimalSoftwareWindow,
    touch: &mut Cst816Touch<'_>,
    last_touch: &mut Option<LogicalPosition>,
    touch_start: &mut Option<(i32, i32)>,
) -> Option<SwipeDirection> {
    match touch.read() {
        Ok(Some(point)) => {
            let position = PhysicalPosition::new(point.x as i32, point.y as i32)
                .to_logical(window.scale_factor());

            if let Some(previous) = last_touch.replace(position) {
                if previous != position {
                    window.dispatch_event(WindowEvent::PointerMoved { position });
                }
            } else {
                touch_start.replace((point.x as i32, point.y as i32));
                window.dispatch_event(WindowEvent::PointerPressed {
                    position,
                    button: PointerEventButton::Left,
                });
            }
        }
        Ok(None) | Err(_) => {
            if let Some(position) = last_touch.take() {
                let end_x = position.x as i32;
                let end_y = position.y as i32;
                let swipe = touch_start.take().and_then(|(start_x, start_y)| {
                    let delta_x = end_x - start_x;
                    let delta_y = end_y - start_y;
                    if delta_x >= 60 && delta_y.abs() <= 100 {
                        Some(SwipeDirection::Right)
                    } else if delta_x <= -60 && delta_y.abs() <= 100 {
                        Some(SwipeDirection::Left)
                    } else {
                        None
                    }
                });

                if swipe.is_some() {
                    crate::esp_debug!("TOUCH: horizontal swipe detected");
                }
                window.dispatch_event(WindowEvent::PointerReleased {
                    position,
                    button: PointerEventButton::Left,
                });
                window.dispatch_event(WindowEvent::PointerExited);
                return swipe;
            }

            touch_start.take();
        }
    }

    None
}
