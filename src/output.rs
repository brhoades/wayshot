use std::{cell::RefCell, fs::File, os::unix::prelude::RawFd, process::exit, rc::Rc};
use wayland_client::{protocol::wl_output, protocol::wl_output::WlOutput};
//, Display, GlobalManager};
// use wayland_protocols::unstable::xdg_output::v1::client::{
//     zxdg_output_manager_v1::ZxdgOutputManagerV1, zxdg_output_v1,
// };
use crate::backend::{FrameCopy, FrameFormat, FrameState};

use wayland_protocols::xdg::xdg_output::zv1::client::{
    zxdg_output_manager_v1, zxdg_output_manager_v1::ZxdgOutputManagerV1, zxdg_output_v1,
    zxdg_output_v1::ZxdgOutputV1,
};

use wayland_protocols_wlr::screencopy::v1::client::{
    zwlr_screencopy_frame_v1, zwlr_screencopy_frame_v1::ZwlrScreencopyFrameV1,
    zwlr_screencopy_manager_v1, zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1,
};

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
