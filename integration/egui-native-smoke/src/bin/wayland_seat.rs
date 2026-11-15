//! Keep a real pointer device on an isolated headless Wayland seat.
//!
//! The test driver supplies absolute motion, primary buttons and wheel input.
//! Every request ends in a real protocol frame. It grants no production facts.

#[cfg(target_os = "linux")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use std::io::BufRead as _;
    use std::time::Instant;

    use wayland_client::protocol::wl_pointer::{Axis, AxisSource, ButtonState};
    use wayland_client::{
        Connection, Dispatch, QueueHandle, delegate_noop, globals, protocol::wl_registry,
    };
    use wayland_protocols_wlr::virtual_pointer::v1::client::{
        zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1,
        zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1,
    };

    struct Seat;
    impl Dispatch<wl_registry::WlRegistry, globals::GlobalListContents> for Seat {
        fn event(
            _: &mut Self,
            _: &wl_registry::WlRegistry,
            _: wl_registry::Event,
            _: &globals::GlobalListContents,
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
        }
    }
    delegate_noop!(Seat: ignore ZwlrVirtualPointerManagerV1);
    delegate_noop!(Seat: ignore ZwlrVirtualPointerV1);

    let connection = Connection::connect_to_env()?;
    let (globals, mut queue) = globals::registry_queue_init::<Seat>(&connection)?;
    let handle = queue.handle();
    let manager = globals.bind::<ZwlrVirtualPointerManagerV1, _, _>(&handle, 1..=2, ())?;
    let pointer = manager.create_virtual_pointer(None, &handle, ());
    queue.roundtrip(&mut Seat)?;
    println!("wayland-seat: virtual pointer ready");
    let started = Instant::now();
    // The isolated driver owns this pipe, bounding the device's lifetime.
    for line in std::io::stdin().lock().lines() {
        let line = line?;
        let mut words = line.split_whitespace();
        let time = u32::try_from(started.elapsed().as_millis())?;
        match words.next() {
            Some("move") => {
                let values = words
                    .map(str::parse::<u32>)
                    .collect::<Result<Vec<_>, _>>()?;
                let [x, y, width, height] = values.as_slice() else {
                    return Err("move requires x, y and coordinate extents".into());
                };
                pointer.motion_absolute(time, *x, *y, *width, *height);
            }
            Some("down") => pointer.button(time, 0x110, ButtonState::Pressed),
            Some("up") => pointer.button(time, 0x110, ButtonState::Released),
            Some("scroll") => {
                pointer.axis_source(AxisSource::Wheel);
                pointer.axis_discrete(time, Axis::VerticalScroll, 15.0, 1);
            }
            _ => return Err("expected move, down, up or scroll".into()),
        }
        pointer.frame();
        connection.flush()?;
    }
    pointer.destroy();
    manager.destroy();
    connection.flush()?;
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    Err("the headless Wayland seat helper requires Linux".into())
}
