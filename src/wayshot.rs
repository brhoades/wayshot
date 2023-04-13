use std::{
    cmp, env,
    error::Error,
    fs::File,
    io::{stdout, BufWriter},
    os::unix::prelude::FromRawFd,
    os::unix::prelude::RawFd,
    process::exit,
    time::{SystemTime, UNIX_EPOCH},
};

use crate::backend::{create_shm_fd, FrameFormat, FrameState};
use crate::convert::create_converter;

use image::{
    imageops::resize, ColorType, GenericImage, ImageBuffer, ImageEncoder, RgbImage, RgbaImage,
};
use memmap2::MmapMut;
use nix::unistd;
use wayland_client::{
    protocol::{
        wl_buffer, wl_buffer::WlBuffer, wl_output, wl_registry, wl_shm, wl_shm::Format,
        wl_shm_pool, wl_shm_pool::WlShmPool,
    },
    Connection, Dispatch, QueueHandle, WEnum,
};
use wayland_protocols::xdg::xdg_output::zv1::client::{
    zxdg_output_manager_v1, zxdg_output_manager_v1::ZxdgOutputManagerV1, zxdg_output_v1,
    zxdg_output_v1::ZxdgOutputV1,
};
use wayland_protocols_wlr::screencopy::v1::client::{
    zwlr_screencopy_frame_v1, zwlr_screencopy_frame_v1::ZwlrScreencopyFrameV1,
    zwlr_screencopy_manager_v1, zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1,
};

mod backend;
mod clap;
mod convert;
mod output;

// TODO: Create a xdg-shell surface, check for the enter event, grab the output from it.
//
// TODO: Patch multiple output bug via multiple images composited into 1.

fn parse_geometry(g: &str) -> Option<backend::CaptureRegion> {
    let tail = g.trim();
    let x_coordinate: i32;
    let y_coordinate: i32;
    let width: i32;
    let height: i32;

    if tail.contains(',') {
        // this accepts: "%d,%d %dx%d"
        let (head, tail) = tail.split_once(',')?;
        x_coordinate = head.parse::<i32>().ok()?;
        let (head, tail) = tail.split_once(' ')?;
        y_coordinate = head.parse::<i32>().ok()?;
        let (head, tail) = tail.split_once('x')?;
        width = head.parse::<i32>().ok()?;
        height = tail.parse::<i32>().ok()?;
    } else {
        // this accepts: "%d %d %d %d"
        let (head, tail) = tail.split_once(' ')?;
        x_coordinate = head.parse::<i32>().ok()?;
        let (head, tail) = tail.split_once(' ')?;
        y_coordinate = head.parse::<i32>().ok()?;
        let (head, tail) = tail.split_once(' ')?;
        width = head.parse::<i32>().ok()?;
        height = tail.parse::<i32>().ok()?;
    }

    Some(backend::CaptureRegion {
        x_coordinate,
        y_coordinate,
        width,
        height,
    })
}

struct WayshotState {
    formats: Vec<wl_shm::Format>,
    outputs: Vec<output::OutputInfo>,
    shm: Option<wl_shm::WlShm>,
    screencopy: Option<ZwlrScreencopyManagerV1>,
    xdg_output: Option<ZxdgOutputManagerV1>,
}

impl Dispatch<wl_registry::WlRegistry, ()> for WayshotState {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<WayshotState>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        {
            match &interface[..] {
                "wl_shm" => {
                    let shm = registry.bind::<wl_shm::WlShm, _, _>(name, 1, qh, ());
                    state.shm = Some(shm);
                }
                "zwlr_screencopy_manager_v1" => {
                    state.screencopy =
                        Some(registry.bind::<ZwlrScreencopyManagerV1, _, _>(name, 1, qh, ()));
                }
                "zxdg_output_manager_v1" => {
                    let manager = registry.bind::<ZxdgOutputManagerV1, _, _>(name, 1, qh, ());
                    for output in state.outputs.iter_mut() {
                        output.xdg_output = Some(manager.get_xdg_output(&output.wl_output, qh, ()));
                    }
                    state.xdg_output = Some(manager);
                }
                "wl_output" => {
                    if version >= 4 {
                        let output = registry.bind::<wl_output::WlOutput, _, _>(name, 4, qh, ());
                        let xdg_output = match &state.xdg_output {
                            Some(manager) => Some(manager.get_xdg_output(&output, qh, ())),
                            None => None,
                        };
                        let info = output::OutputInfo {
                            wl_output: output,
                            name: "".to_string(),
                            xdg_output,
                            dimensions: output::OutputPositioning {
                                x: 0,
                                y: 0,
                                width: 0,
                                height: 0,
                            },
                            xdg_ready: false,
                            wl_ready: false,
                            frame: None,
                            frame_state: None,
                            frame_format: None,
                            mem_fd: None,
                        };
                        state.outputs.push(info);
                    }
                }
                _ => {}
            }
        }
    }
}

impl Dispatch<wl_output::WlOutput, ()> for WayshotState {
    fn event(
        state: &mut Self,
        wl_output: &wl_output::WlOutput,
        event: wl_output::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        for output in state.outputs.iter_mut() {
            if output.wl_output != *wl_output {
                continue;
            }

            if let wl_output::Event::Name { name } = &event {
                output.name = name.clone();
            }
            if let wl_output::Event::Done {} = &event {
                output.wl_ready = true;
            }
        }
    }
}

impl Dispatch<ZxdgOutputV1, ()> for WayshotState {
    fn event(
        state: &mut Self,
        xdg_output: &zxdg_output_v1::ZxdgOutputV1,
        event: zxdg_output_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        for output in state.outputs.iter_mut() {
            let xdgo = if let Some(xdgo) = &output.xdg_output {
                xdgo
            } else {
				continue;
			};
            if xdgo != xdg_output {
                continue;
            }

            if let zxdg_output_v1::Event::LogicalPosition { x, y } = &event {
                output.dimensions.x = *x;
                output.dimensions.y = *y;
            }

            if let zxdg_output_v1::Event::LogicalSize { width, height } = &event {
                output.dimensions.width = *width;
                output.dimensions.height = *height;
            }
            if let zxdg_output_v1::Event::Done = &event {
                // todo: atomically apply queued position/size; this will
                // avoid a race condition
                output.xdg_ready = true;
            }
        }
    }
}

impl Dispatch<wl_shm::WlShm, ()> for WayshotState {
    fn event(
        state: &mut Self,
        _: &wl_shm::WlShm,
        event: wl_shm::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_shm::Event::Format { format } = event {
            if let WEnum::Value(v) = format {
                state.formats.push(v)
            };
        }
    }
}

impl Dispatch<ZxdgOutputManagerV1, ()> for WayshotState {
    fn event(
        _: &mut Self,
        _: &ZxdgOutputManagerV1,
        _: zxdg_output_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwlrScreencopyManagerV1, ()> for WayshotState {
    fn event(
        _: &mut Self,
        _: &ZwlrScreencopyManagerV1,
        _: zwlr_screencopy_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwlrScreencopyFrameV1, ()> for WayshotState {
    fn event(
        state: &mut Self,
        frame: &ZwlrScreencopyFrameV1,
        event: zwlr_screencopy_frame_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        for output in state.outputs.iter_mut() {
            let f = if let Some(f) = &output.frame {
                f
            } else {
				continue;
			};
            if f != frame {
                continue;
            }

            match event {
                zwlr_screencopy_frame_v1::Event::Buffer {
                    format,
                    width,
                    height,
                    stride,
                } => {
                    log::debug!("Received Buffer event");
                    output.frame_format = Some(FrameFormat {
                        format: format.into_result().unwrap(),
                        width,
                        height,
                        stride,
                    });
                }
                zwlr_screencopy_frame_v1::Event::Flags { .. } => {
                    log::debug!("Received Flags event");
                }
                zwlr_screencopy_frame_v1::Event::Ready { .. } => {
                    // If the frame is successfully copied, a “flags” and a “ready” events are sent. Otherwise, a “failed” event is sent.
                    // This is useful when we call .copy on the frame object.
                    log::debug!("Received Ready event");
                    output.frame_state = Some(FrameState::Finished)
                }
                zwlr_screencopy_frame_v1::Event::Failed => {
                    log::debug!("Received Failed event");
                    output.frame_state = Some(FrameState::Failed);
                }
                zwlr_screencopy_frame_v1::Event::Damage { .. } => {
                    log::debug!("Received Damage event");
                }
                zwlr_screencopy_frame_v1::Event::LinuxDmabuf { .. } => {
                    log::debug!("Received LinuxDmaBuf event");
                }
                zwlr_screencopy_frame_v1::Event::BufferDone => {
                    log::debug!("Received bufferdone event");
                    // todo: verify this arrived
                }
                _ => unreachable!(),
            };
        }
    }
}

impl Dispatch<WlBuffer, ()> for WayshotState {
    fn event(
        _: &mut Self,
        _: &WlBuffer,
        _: wl_buffer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlShmPool, ()> for WayshotState {
    fn event(
        _: &mut Self,
        _: &WlShmPool,
        _: wl_shm_pool::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let args = clap::set_flags().get_matches();
    env::set_var("RUST_LOG", "wayshot=info");

    if args.is_present("debug") {
        env::set_var("RUST_LOG", "wayshot=trace");
    }

    env_logger::init();
    log::trace!("Logger initialized.");

    let cursor_overlay: i32 = if args.is_present("cursor") { 1 } else { 0 };

    let mut state = WayshotState {
        outputs: Vec::new(),
        shm: None,
        screencopy: None,
        xdg_output: None,
        formats: Vec::new(),
    };
    let conn = wayland_client::Connection::connect_to_env().unwrap();
    let display = conn.display();

    let mut event_queue = conn.new_event_queue();
    let qh: QueueHandle<WayshotState> = event_queue.handle();
    // todo: use the registry abstraction from wayland-client
    let _registry = display.get_registry(&qh, ());

    // First roundtrip: bind all globals and outputs
    event_queue.roundtrip(&mut state).unwrap();
    if state.shm.is_none() {
        log::error!("Compositor is missing wl_shm interface");
        exit(1);
    }
    if state.shm.is_none() {
        log::error!("Compositor is missing wl_shm interface");
        exit(1);
    }

    // Second roundtrip: learn output names and geometry
    event_queue.roundtrip(&mut state).unwrap();

    if args.is_present("listoutputs") {
        for output in state.outputs {
            if output.wl_ready {
                log::info!("{:#?}", output.name);
            } else {
                log::error!("An output did not report its name");
            }
        }
        exit(1);
    }

    // If an output is chosen, select only it
    if let Some(chosen_output) = args.value_of("output") {
        // Remove all outputs which do not match
        state.outputs = state
            .outputs
            .into_iter()
            .filter(|output| output.wl_ready && output.name == chosen_output)
            .collect();
        // todo: impl drop?
    }

    let region = if let Some(slurpval) = args.value_of("slurp") {
        if slurpval == "" {
            log::error!("Failed to recieve geometry.");
            exit(1);
        }
        let region: backend::CaptureRegion =
            parse_geometry(slurpval).expect("Invalid geometry specification");
        region
    } else {
        backend::CaptureRegion {
            // with signed integers, x_1,x_2,y_1,y_2 is better structure
            x_coordinate: i32::MIN / 2,
            y_coordinate: i32::MIN / 2,
            width: i32::MAX,
            height: i32::MAX,
        }
    };

    // Remove all outputs which do not overlap the target region
    state.outputs = state
        .outputs
        .into_iter()
        .filter(|output| {
            let x1: i32 = cmp::max(output.dimensions.x, region.x_coordinate);
            let y1: i32 = cmp::max(output.dimensions.y, region.y_coordinate);
            let x2: i32 = cmp::min(
                output.dimensions.x + output.dimensions.width,
                region.x_coordinate + region.width,
            );
            let y2: i32 = cmp::min(
                output.dimensions.y + output.dimensions.height,
                region.y_coordinate + region.height,
            );

            let width = x2 - x1;
            let height = y2 - y1;
            width > 0 && height > 0
        })
        .collect();

    if state.outputs.is_empty() {
        log::error!("Provided capture region doesn't intersect with any outputs!");
        exit(1);
    }

    let mut net_x1: i32 = i32::MAX;
    let mut net_x2: i32 = i32::MIN;
    let mut net_y1: i32 = i32::MAX;
    let mut net_y2: i32 = i32::MIN;
    for output in state.outputs.iter_mut() {
        let manager = state.screencopy.as_mut().unwrap();

        let x1: i32 = cmp::max(output.dimensions.x, region.x_coordinate);
        let y1: i32 = cmp::max(output.dimensions.y, region.y_coordinate);
        let x2: i32 = cmp::min(
            output.dimensions.x + output.dimensions.width,
            region.x_coordinate + region.width,
        );
        let y2: i32 = cmp::min(
            output.dimensions.y + output.dimensions.height,
            region.y_coordinate + region.height,
        );

        net_x1 = cmp::min(net_x1, x1);
        net_x2 = cmp::max(net_x2, x2);
        net_y1 = cmp::min(net_y1, y1);
        net_y2 = cmp::max(net_y2, y2);

        // Quoting spec: "The region is given in output logical coordinates"
        // So subtract output position from global logical coordinates
        let frame = manager.capture_output_region(
            cursor_overlay,
            &output.wl_output,
            x1 - output.dimensions.x,
            y1 - output.dimensions.y,
            x2 - x1,
            y2 - y1,
            &qh,
            (),
        );
        output.frame = Some(frame);
    }

    // Third roundtrip: learn frame parameters for requests
    event_queue.roundtrip(&mut state).unwrap();

    for output in state.outputs.iter_mut() {
        let shm = state.shm.as_mut().unwrap();

        let frame_format = if let Some(frame_format) = output.frame_format {
            frame_format
        } else {
            log::error!("Output did not specify a frame format");
            exit(1);
        };

        let frame_bytes = frame_format.stride * frame_format.height;

        // Create an in memory file and return it's file descriptor.
        let mem_fd = create_shm_fd()?;
        unistd::ftruncate(mem_fd, frame_bytes as i64).unwrap();
        output.mem_fd = Some(mem_fd);

        let shm_pool = shm.create_pool(mem_fd, frame_bytes as i32, &qh, ());
        let buffer = shm_pool.create_buffer(
            0,
            frame_format.width as i32,
            frame_format.height as i32,
            frame_format.stride as i32,
            frame_format.format,
            &qh,
            (),
        );

        // Copy the pixel data advertised by the compositor into the buffer we just created.
        output.frame.as_mut().unwrap().copy(&buffer);
    }

    // Fourth roundtrip: learn whether captures succeeded or failed.
    loop {
        // todo: how to dispatch?
        event_queue.roundtrip(&mut state).unwrap();
        if !state
            .outputs
            .iter()
            .any(|output| output.frame_state.is_none())
        {
            break;
        }
    }

    // Process and save outputs. (At the moment, just dump captures to files, using distinct names
    // if there are many.)
    let extension = if args.is_present("extension") {
        let ext: &str = &args.value_of("extension").unwrap().trim().to_lowercase();
        match ext {
            "jpeg" | "jpg" => backend::EncodingFormat::Jpg,
            "png" => backend::EncodingFormat::Png,
            "ppm" => backend::EncodingFormat::Ppm,
            _ => {
                log::error!("Invalid extension provided.\nValid extensions:\n1) jpeg\n2) jpg\n3) png\n4) ppm");
                exit(1);
            }
        }
    } else {
        backend::EncodingFormat::Png
    };

    if extension != backend::EncodingFormat::Png {
        log::debug!("Using custom extension: {:#?}", extension);
    }

    // TODO: render at 2x or higher scale later? Default should probably be >2x
    // max fractional scale, or something close to a rational multiple of all outputs
    let dest_width = (net_x2 - net_x1) as u32;
    let dest_height = (net_y2 - net_y1) as u32;
    let mut dest: RgbaImage = ImageBuffer::new(dest_width, dest_height);

    for output in state.outputs.iter_mut() {
        match output.frame_state {
            None => unreachable!(),
            Some(FrameState::Failed) => {
                log::error!("Frame copy failed");
                exit(1);
            }
            Some(FrameState::Finished) => {
                let mem_fd = output.mem_fd.unwrap();

                let frame_format = output.frame_format.unwrap();
                let frame_bytes = frame_format.stride * frame_format.height;

                let mem_file = unsafe { File::from_raw_fd(mem_fd) };
                let mut frame_mmap = unsafe { MmapMut::map_mut(&mem_file)? };
                let data = &mut *frame_mmap;
                let frame_color_type = if let Some(converter) =
                    create_converter(frame_format.format)
                {
                    converter.convert_inplace(data)
                } else {
                    log::error!("Unsupported buffer format: {:?}", frame_format.format);
                    log::error!("You can send a feature request for the above format to the mailing list for wayshot over at https://sr.ht/~shinyzenith/wayshot.");
                    exit(1);
                };
                let frame_image = RgbaImage::from_raw(
                    frame_format.width,
                    frame_format.height,
                    (&*frame_mmap).to_vec(),
                )
                .unwrap();

                let x1: i32 = cmp::max(output.dimensions.x, region.x_coordinate);
                let y1: i32 = cmp::max(output.dimensions.y, region.y_coordinate);
                let x2: i32 = cmp::min(
                    output.dimensions.x + output.dimensions.width,
                    region.x_coordinate + region.width,
                );
                let y2: i32 = cmp::min(
                    output.dimensions.y + output.dimensions.height,
                    region.y_coordinate + region.height,
                );

                let resized: RgbaImage = resize(
                    &frame_image,
                    (x2 - x1) as u32,
                    (y2 - y1) as u32,
                    image::imageops::FilterType::Triangle,
                );
                if let Err(e) = dest.copy_from(&resized, (x1 - net_x1) as u32, (y1 - net_y1) as u32)
                {
                    log::error!("Failed to copy output image onto dest image: {:?}", e);
                    exit(1);
                }

                // todo: cleanup?
            }
        }
    }

    if args.is_present("stdout") {
        let stdout = stdout();
        let writer = BufWriter::new(stdout.lock());
        backend::write_to_file(writer, extension, dest)?;
    } else {
        let path = if args.is_present("file") {
            args.value_of("file").unwrap().trim().to_string()
        } else {
            let time = match SystemTime::now().duration_since(UNIX_EPOCH) {
                Ok(n) => n.as_secs().to_string(),
                Err(_) => {
                    log::error!("SystemTime before UNIX EPOCH!");
                    exit(1);
                }
            };

            time + match extension {
                backend::EncodingFormat::Png => "-wayshot.png",
                backend::EncodingFormat::Jpg => "-wayshot.jpg",
                backend::EncodingFormat::Ppm => "-wayshot.ppm",
            }
        };

        backend::write_to_file(File::create(path)?, extension, dest)?;
    }

    Ok(())
}
