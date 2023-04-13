use std::{
    cell::RefCell,
    error::Error,
    ffi::CStr,
    fs::File,
    io::Write,
    os::unix::prelude::FromRawFd,
    os::unix::prelude::RawFd,
    process::exit,
    rc::Rc,
    sync::atomic::{AtomicBool, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use nix::{
    fcntl,
    sys::{memfd, mman, stat},
    unistd,
};

use image::{
    codecs::{
        jpeg::JpegEncoder,
        png::PngEncoder,
        pnm::{self, PnmEncoder},
    },
    ColorType, ImageEncoder, RgbaImage,
};
use memmap2::MmapMut;

use wayland_client::protocol::{wl_output::WlOutput, wl_shm, wl_shm::Format};
/*use wayland_protocols::wlr::unstable::screencopy::v1::client::{
    zwlr_screencopy_frame_v1, zwlr_screencopy_frame_v1::ZwlrScreencopyFrameV1,
    zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1,
};*/

use crate::convert::create_converter;

/// Type of frame supported by the compositor. For now we only support Argb8888, Xrgb8888, and
/// Xbgr8888.
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct FrameFormat {
    pub format: Format,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
}

/// State of the frame after attemting to copy it's data to a wl_buffer.
#[derive(Debug, Copy, Clone, PartialEq)]
pub enum FrameState {
    /// Compositor returned a failed event on calling `frame.copy`.
    Failed,
    /// Compositor sent a Ready event on calling `frame.copy`.
    Finished,
}

/// The copied frame comprising of the FrameFormat, ColorType (Rgba8), and a memory backed shm
/// file that holds the image data in it.
#[derive(Debug)]
pub struct FrameCopy {
    pub frame_format: FrameFormat,
    pub frame_color_type: ColorType,
    pub frame_mmap: MmapMut,
}

/// Struct to store region capture details.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct CaptureRegion {
    /// X coordinate of the area to capture.
    pub x_coordinate: i32,
    /// y coordinate of the area to capture.
    pub y_coordinate: i32,
    /// Width of the capture area.
    pub width: i32,
    /// Height of the capture area.
    pub height: i32,
}

/// Supported image encoding formats.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum EncodingFormat {
    /// Jpeg / jpg encoder.
    Jpg,
    /// Png encoder.
    Png,
    /// Ppm encoder
    Ppm,
}

/// Return a RawFd to a shm file. We use memfd create on linux and shm_open for BSD support.
/// You don't need to mess around with this function, it is only used by
/// capture_output_frame.
pub fn create_shm_fd() -> std::io::Result<RawFd> {
    // Only try memfd on linux and freebsd.
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    loop {
        // Create a file that closes on succesful execution and seal it's operations.
        match memfd::memfd_create(
            CStr::from_bytes_with_nul(b"wayshot\0").unwrap(),
            memfd::MemFdCreateFlag::MFD_CLOEXEC | memfd::MemFdCreateFlag::MFD_ALLOW_SEALING,
        ) {
            Ok(fd) => {
                // This is only an optimization, so ignore errors.
                // F_SEAL_SRHINK = File cannot be reduced in size.
                // F_SEAL_SEAL = Prevent further calls to fcntl().
                let _ = fcntl::fcntl(
                    fd,
                    fcntl::F_ADD_SEALS(
                        fcntl::SealFlag::F_SEAL_SHRINK | fcntl::SealFlag::F_SEAL_SEAL,
                    ),
                );
                return Ok(fd);
            }
            Err(nix::errno::Errno::EINTR) => continue,
            Err(nix::errno::Errno::ENOSYS) => break,
            Err(errno) => return Err(std::io::Error::from(errno)),
        }
    }

    // Fallback to using shm_open.
    let sys_time = SystemTime::now();
    let mut mem_file_handle = format!(
        "/wayshot-{}",
        sys_time.duration_since(UNIX_EPOCH).unwrap().subsec_nanos()
    );
    loop {
        match mman::shm_open(
            // O_CREAT = Create file if does not exist.
            // O_EXCL = Error if create and file exists.
            // O_RDWR = Open for reading and writing.
            // O_CLOEXEC = Close on succesful execution.
            // S_IRUSR = Set user read permission bit .
            // S_IWUSR = Set user write permission bit.
            mem_file_handle.as_str(),
            fcntl::OFlag::O_CREAT
                | fcntl::OFlag::O_EXCL
                | fcntl::OFlag::O_RDWR
                | fcntl::OFlag::O_CLOEXEC,
            stat::Mode::S_IRUSR | stat::Mode::S_IWUSR,
        ) {
            Ok(fd) => match mman::shm_unlink(mem_file_handle.as_str()) {
                Ok(_) => return Ok(fd),
                Err(errno) => match unistd::close(fd) {
                    Ok(_) => return Err(std::io::Error::from(errno)),
                    Err(errno) => return Err(std::io::Error::from(errno)),
                },
            },
            Err(nix::errno::Errno::EEXIST) => {
                // If a file with that handle exists then change the handle
                mem_file_handle = format!(
                    "/wayshot-{}",
                    sys_time.duration_since(UNIX_EPOCH).unwrap().subsec_nanos()
                );
                continue;
            }
            Err(nix::errno::Errno::EINTR) => continue,
            Err(errno) => return Err(std::io::Error::from(errno)),
        }
    }
}

/// Write an instance of FrameCopy to anything that implements Write trait. Eg: Stdout or a file
/// on the disk.
pub fn write_to_file(
    mut output_file: impl Write,
    encoding_format: EncodingFormat,
    image: RgbaImage,
) -> Result<(), Box<dyn Error>> {
    log::debug!(
        "Writing to disk with encoding format: {:#?}",
        encoding_format
    );
    match encoding_format {
        EncodingFormat::Jpg => {
            JpegEncoder::new(&mut output_file).write_image(
                image.as_raw(),
                image.width(),
                image.height(),
                ColorType::Rgba8,
            )?;
            output_file.flush()?;
        }
        EncodingFormat::Png => {
            PngEncoder::new(&mut output_file).write_image(
                image.as_raw(),
                image.width(),
                image.height(),
                ColorType::Rgba8,
            )?;
            output_file.flush()?;
        }
        EncodingFormat::Ppm => {
            let rgb8_data = {
                let mut data = Vec::with_capacity((3 * image.width() * image.height()) as _);

                for chunk in image.as_raw().chunks_exact(4) {
                    data.extend_from_slice(&chunk[..3]);
                }
                data
            };

            PnmEncoder::new(&mut output_file)
                .with_subtype(pnm::PnmSubtype::Pixmap(pnm::SampleEncoding::Binary))
                .write_image(&rgb8_data, image.width(), image.height(), ColorType::Rgb8)?;
            output_file.flush()?;
        }
    }

    Ok(())
}
