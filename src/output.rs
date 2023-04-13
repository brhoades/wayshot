use crate::backend::{FrameFormat, FrameState};
use std::os::unix::prelude::RawFd;
use wayland_client::protocol::wl_output::WlOutput;

use wayland_protocols::xdg::xdg_output::zv1::client::zxdg_output_v1::ZxdgOutputV1;

use wayland_protocols_wlr::screencopy::v1::client::zwlr_screencopy_frame_v1::ZwlrScreencopyFrameV1;

#[derive(Debug, Clone)]
pub struct OutputInfo {
    pub wl_output: WlOutput,
    pub xdg_output: Option<ZxdgOutputV1>,
    pub name: String,
    pub dimensions: OutputPositioning,
    pub xdg_ready: bool, // has received ZxdgOutputV1::Event::Done
    pub wl_ready: bool,  // has received WlOutput::Event::Done
    pub frame: Option<ZwlrScreencopyFrameV1>,
    pub frame_state: Option<FrameState>,
    pub frame_format: Option<FrameFormat>,
    pub mem_fd: Option<RawFd>,
}

#[derive(Default, Debug, Clone, PartialEq, Eq)]
pub struct OutputPositioning {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}
