//! Kitty graphics image loading (`ZIG/src/terminal/kitty/graphics_image.zig`).
//!
//! [`LoadingImage`] is an image that is still being transmitted (possibly in
//! chunks). [`Image`] is a fully loaded and validated image.

#[cfg(unix)]
use std::os::unix::{ffi::OsStrExt, fs::FileExt};
use std::{
    io::{Cursor, Read, Seek, SeekFrom},
    path::Path,
    time::Instant,
};

/// wasm32 has no `std::os::unix`; on Windows `as_encoded_bytes` is WTF-8.
#[cfg(not(unix))]
trait OsStrExt {
    fn as_bytes(&self) -> &[u8];
}

#[cfg(not(unix))]
impl OsStrExt for std::ffi::OsStr {
    fn as_bytes(&self) -> &[u8] {
        self.as_encoded_bytes()
    }
}

/// The OS path named by protocol payload bytes. Unix paths are the bytes;
/// on Windows they are WTF-8 (lone surrogates are valid, malformed input is
/// not a path); elsewhere (wasm) they must be UTF-8.
#[cfg(unix)]
#[allow(clippy::unnecessary_wraps)] // same signature as the fallible targets
fn os_path_from_bytes(bytes: &[u8]) -> Option<std::ffi::OsString> {
    Some(std::ffi::OsStr::from_bytes(bytes).to_owned())
}

#[cfg(windows)]
fn os_path_from_bytes(bytes: &[u8]) -> Option<std::ffi::OsString> {
    ghostty_os::osbytes::EncodedOs::from_bytes(bytes)
        .try_to_os()
        .ok()
}

#[cfg(not(any(unix, windows)))]
fn os_path_from_bytes(bytes: &[u8]) -> Option<std::ffi::OsString> {
    std::str::from_utf8(bytes).ok().map(Into::into)
}

/// `posix.realpath` (no `\\?\` verbatim prefix on Windows, like Zig).
#[cfg(windows)]
fn realpath(path: impl AsRef<Path>) -> std::io::Result<std::path::PathBuf> {
    ghostty_os::path::realpath(path)
}

#[cfg(not(windows))]
fn realpath(path: impl AsRef<Path>) -> std::io::Result<std::path::PathBuf> {
    std::fs::canonicalize(path)
}

use ghostty_vt::kitty::graphics_command::{
    Command,
    Display,
    Quiet,
    TransmissionCompression,
    TransmissionFormat,
    TransmissionMedium,
};

use crate::page_list::Pin;

/// Maximum width or height of an image. Taken directly from Kitty.
pub const MAX_DIMENSION: u32 = 10000;

/// Maximum size in bytes, taken from Kitty.
pub const MAX_SIZE: usize = 400 * 1024 * 1024; // 400MB

/// Errors that can occur while loading an image (`Image.Error` plus
/// `OutOfMemory` in Zig).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("out of memory")]
    OutOfMemory,
    #[error("internal error")]
    InternalError,
    #[error("invalid data")]
    InvalidData,
    #[error("decompression failed")]
    DecompressionFailed,
    #[error("dimensions required")]
    DimensionsRequired,
    #[error("dimensions too large")]
    DimensionsTooLarge,
    #[error("file path too long")]
    FilePathTooLong,
    #[error("temporary file not in temp dir")]
    TemporaryFileNotInTempDir,
    #[error("temporary file not named correctly")]
    TemporaryFileNotNamedCorrectly,
    #[error("unsupported format")]
    UnsupportedFormat,
    #[error("unsupported medium")]
    UnsupportedMedium,
    #[error("unsupported pixel depth")]
    UnsupportedDepth,
}

/// An image that is still being loaded. The image should be initialized
/// using [`LoadingImage::init`] on the first chunk and then `add_data` for
/// each subsequent chunk. Once all chunks have been added, `complete`
/// should be called to finalize the image.
#[derive(Debug)]
pub struct LoadingImage {
    /// The in-progress image. The first chunk must have all the metadata
    /// so this comes from that initially.
    pub image: Image,

    /// The data that is being built up.
    pub data: Vec<u8>,

    /// This is non-null when a transmit and display command is given
    /// so that we display the image after it is fully loaded.
    pub display: Option<Display>,

    /// Quiet is the quiet settings for the initial load command. This is
    /// used if q isn't set on subsequent chunks.
    pub quiet: Quiet,
}

impl LoadingImage {
    /// Initialize a chunked image from the first image transmission.
    /// If this is a multi-chunk image, this should only be the FIRST
    /// chunk.
    pub fn init(cmd: &Command) -> Result<LoadingImage, Error> {
        // Build our initial image from the properties sent via the control.
        // These can be overwritten by the data loading process. For example,
        // PNG loading sets the width/height from the data.
        let t = cmd.transmission().ok_or(Error::InternalError)?;
        let mut result = LoadingImage {
            image: Image {
                id: t.image_id,
                number: t.image_number,
                width: t.width,
                height: t.height,
                compression: t.compression,
                format: t.format,
                ..Image::default()
            },
            data: Vec::new(),
            display: cmd.display(),
            quiet: cmd.quiet,
        };

        // Special case for the direct medium, we just add the chunk directly.
        if t.medium == TransmissionMedium::Direct {
            result.add_data(&cmd.data)?;
            return Ok(result);
        }

        // Otherwise, the payload data is guaranteed to be a path.
        if cmd.data.contains(&0) {
            log::warn!("failed to get absolute path: BadPathName");
            return Err(Error::InvalidData);
        }

        match t.medium {
            TransmissionMedium::Direct => unreachable!("handled above"),
            TransmissionMedium::File | TransmissionMedium::TemporaryFile => {
                let Some(os_path) = os_path_from_bytes(&cmd.data) else {
                    log::warn!("failed to get absolute path: BadPathName");
                    return Err(Error::InvalidData);
                };
                let path = match realpath(Path::new(&os_path)) {
                    Ok(p) => p,
                    Err(err) => {
                        log::warn!("failed to get absolute path: {err}");
                        return Err(Error::InvalidData);
                    }
                };
                result.read_file(
                    t.medium == TransmissionMedium::TemporaryFile,
                    t.offset,
                    t.size,
                    path.as_os_str().as_bytes(),
                )?;
            }
            TransmissionMedium::SharedMemory => {
                result.read_shared_memory(t.offset, t.size, &cmd.data)?;
            }
        }

        Ok(result)
    }

    /// Reads the data from a shared memory segment.
    #[cfg(target_arch = "wasm32")]
    #[allow(clippy::unused_self)]
    fn read_shared_memory(&mut self, _offset: u32, _size: u32, path: &[u8]) -> Result<(), Error> {
        // POSIX shared memory does not exist on wasm32.
        log::warn!(
            "shared memory not supported on wasm: {}",
            String::from_utf8_lossy(path)
        );
        Err(Error::InvalidData)
    }

    /// Reads the data from a shared memory segment.
    ///
    /// Windows is currently unsupported (as in Zig).
    #[cfg(windows)]
    #[allow(clippy::unused_self)]
    fn read_shared_memory(&mut self, _offset: u32, _size: u32, _path: &[u8]) -> Result<(), Error> {
        Err(Error::UnsupportedMedium)
    }

    /// Reads the data from a shared memory segment.
    #[cfg(not(any(target_arch = "wasm32", windows)))]
    fn read_shared_memory(&mut self, offset: u32, size: u32, path: &[u8]) -> Result<(), Error> {
        use rustix::{fs::Mode, shm};

        let path_str = String::from_utf8_lossy(path);
        let fd = match shm::open(path, shm::OFlags::RDONLY, Mode::empty()) {
            Ok(fd) => fd,
            Err(err) => {
                log::warn!("unable to open shared memory {path_str}: {err}");
                return Err(Error::InvalidData);
            }
        };
        // The segment is always unlinked once we're done (even on error).
        let result = self.read_shm_fd(fd, offset, size, &path_str);
        let _ = shm::unlink(path);
        result
    }

    #[cfg(not(any(target_arch = "wasm32", windows)))]
    fn read_shm_fd(
        &mut self,
        fd: rustix::fd::OwnedFd,
        offset: u32,
        size: u32,
        path: &str,
    ) -> Result<(), Error> {
        let file = std::fs::File::from(fd);

        // The size from stat on may be larger than our expected size because
        // shared memory has to be a multiple of the page size.
        let stat_size: usize = match file.metadata() {
            Ok(m) => {
                if m.len() == 0 {
                    return Err(Error::InvalidData);
                }
                m.len() as usize
            }
            Err(err) => {
                log::warn!("unable to fstat shared memory {path}: {err}");
                return Err(Error::InvalidData);
            }
        };

        let expected_size: usize = match self.image.format {
            // Png we decode the full data size because later decoding will
            // get the proper dimensions and assert validity.
            TransmissionFormat::Png => stat_size,

            // For these formats we have a size we must have.
            f => {
                let bpp = usize::from(f.bpp());
                (self.image.width as usize)
                    .saturating_mul(self.image.height as usize)
                    .saturating_mul(bpp)
            }
        };

        // Our stat size must be at least the expected size otherwise
        // the shared memory data is invalid.
        if stat_size < expected_size {
            log::warn!("shared memory size too small expected={expected_size} actual={stat_size}");
            return Err(Error::InvalidData);
        }

        // Our end size always uses the expected size so we cut off the
        // padding for mmap alignment.
        let start = offset as usize;
        let end = if size > 0 {
            (start + size as usize).min(expected_size)
        } else {
            expected_size
        };
        if start > end {
            return Err(Error::InvalidData);
        }

        debug_assert_eq!(self.data.len(), 0);
        let mut buf = vec![0u8; end - start];
        if let Err(err) = file.read_exact_at(&mut buf, start as u64) {
            log::warn!("unable to read shared memory {path}: {err}");
            return Err(Error::InvalidData);
        }
        self.data = buf;
        Ok(())
    }

    /// Reads the data from a (temporary) file.
    ///
    /// This will also delete the temporary file if it is in a safe location.
    fn read_file(
        &mut self,
        temporary: bool,
        offset: u32,
        size: u32,
        path: &[u8],
    ) -> Result<(), Error> {
        // Verify file seems "safe". This is logic copied directly from Kitty,
        // mostly. This is really rough but it will catch obvious bad actors.
        if path.starts_with(b"/proc/")
            || path.starts_with(b"/sys/")
            || (path.starts_with(b"/dev/") && !path.starts_with(b"/dev/shm/"))
        {
            return Err(Error::InvalidData);
        }

        // Temporary file logic
        if temporary {
            if !is_path_in_temp_dir(path) {
                return Err(Error::TemporaryFileNotInTempDir);
            }
            if !contains_subslice(path, b"tty-graphics-protocol") {
                return Err(Error::TemporaryFileNotNamedCorrectly);
            }
        }

        let Some(os_path) = os_path_from_bytes(path) else {
            return Err(Error::InvalidData);
        };
        let os_path = Path::new(&os_path);
        let result = self.read_file_inner(os_path, offset, size);
        if temporary && let Err(err) = std::fs::remove_file(os_path) {
            log::warn!("failed to delete temporary file: {err}");
        }
        result
    }

    fn read_file_inner(&mut self, path: &Path, offset: u32, size: u32) -> Result<(), Error> {
        let mut file = match std::fs::File::open(path) {
            Ok(f) => f,
            Err(err) => {
                log::warn!("failed to open temporary file: {err}");
                return Err(Error::InvalidData);
            }
        };

        // File must be a regular file
        match file.metadata() {
            Ok(m) => {
                if !m.is_file() {
                    log::warn!("file is not a regular file");
                    return Err(Error::InvalidData);
                }
            }
            Err(err) => {
                log::warn!("failed to stat file: {err}");
                return Err(Error::InvalidData);
            }
        }

        if offset > 0
            && let Err(err) = file.seek(SeekFrom::Start(u64::from(offset)))
        {
            log::warn!("failed to seek to offset {offset}: {err}");
            return Err(Error::InvalidData);
        }

        // Read the file
        let limit = if size > 0 {
            (size as usize).min(MAX_SIZE)
        } else {
            MAX_SIZE
        };
        let mut buf = Vec::new();
        if let Err(err) = file.take(limit as u64).read_to_end(&mut buf) {
            log::warn!("failed to read temporary file: {err}");
            return Err(Error::InvalidData);
        }

        debug_assert_eq!(self.data.len(), 0);
        self.data = buf;
        Ok(())
    }

    /// The file name `debug_dump` writes (Zig `@tagName` spellings).
    #[cfg(debug_assertions)]
    #[must_use]
    pub fn debug_dump_filename(&self) -> String {
        let format = match self.image.format {
            TransmissionFormat::Rgb => "rgb",
            TransmissionFormat::Rgba => "rgba",
            TransmissionFormat::Png => "png",
            TransmissionFormat::GrayAlpha => "gray_alpha",
            TransmissionFormat::Gray => "gray",
        };
        let compression = match self.image.compression {
            TransmissionCompression::None => "none",
            TransmissionCompression::ZlibDeflate => "zlib_deflate",
        };
        format!(
            "image-{format}-{compression}-{}x{}-{}.data",
            self.image.width, self.image.height, self.image.id
        )
    }

    /// Debug function to write the data to a file in `dir`. This is useful
    /// for capturing some test data for unit tests. Only exists in debug
    /// builds (Zig: `@compileError("debugDump in non-debug")`).
    #[cfg(debug_assertions)]
    pub fn debug_dump_to(&self, dir: &std::path::Path) -> std::io::Result<()> {
        std::fs::write(dir.join(self.debug_dump_filename()), &self.data)
    }

    /// Same as [`Self::debug_dump_to`] using the current working directory,
    /// like Zig's `debugDump`.
    #[cfg(debug_assertions)]
    pub fn debug_dump(&self) -> std::io::Result<()> {
        self.debug_dump_to(std::path::Path::new("."))
    }

    /// Adds a chunk of data to the image. Use this if the image
    /// is coming in chunks (the "m" parameter in the protocol).
    pub fn add_data(&mut self, data: &[u8]) -> Result<(), Error> {
        // If no data, skip
        if data.is_empty() {
            return Ok(());
        }

        // If our data would get too big, return an error
        if self.data.len() + data.len() > MAX_SIZE {
            log::warn!("image data too large max_size={MAX_SIZE}");
            return Err(Error::InvalidData);
        }

        self.data.extend_from_slice(data);
        Ok(())
    }

    /// Complete the chunked image, returning a completed image.
    pub fn complete(&mut self) -> Result<Image, Error> {
        // Decompress the data if it is compressed.
        self.decompress()?;

        // Decode the png if we have to
        if self.image.format == TransmissionFormat::Png {
            self.decode_png()?;
        }

        let img = &self.image;

        // Validate our dimensions.
        if img.width == 0 || img.height == 0 {
            return Err(Error::DimensionsRequired);
        }
        if img.width > MAX_DIMENSION || img.height > MAX_DIMENSION {
            return Err(Error::DimensionsTooLarge);
        }

        // Data length must be what we expect
        let bpp = usize::from(img.format.bpp());
        let expected_len = img.width as usize * img.height as usize * bpp;
        let actual_len = self.data.len();
        if actual_len != expected_len {
            log::warn!(
                "unexpected length image id={} width={} height={} bpp={} expected_len={} actual_len={}",
                img.id,
                img.width,
                img.height,
                bpp,
                expected_len,
                actual_len
            );
            return Err(Error::InvalidData);
        }

        // Set our time
        self.image.transmit_time = Instant::now();

        // Everything looks good, copy the image data over.
        let mut result = std::mem::take(&mut self.image);
        result.data = std::mem::take(&mut self.data);
        Ok(result)
    }

    /// Decompress the data in-place.
    fn decompress(&mut self) -> Result<(), Error> {
        match self.image.compression {
            TransmissionCompression::None => Ok(()),
            TransmissionCompression::ZlibDeflate => self.decompress_zlib(),
        }
    }

    fn decompress_zlib(&mut self) -> Result<(), Error> {
        let decoder = flate2::read::ZlibDecoder::new(self.data.as_slice());
        let mut list = Vec::new();
        // Read one byte past the limit so that an overlong stream is an
        // error (Zig: `.limited(max_size)` + StreamTooLong).
        if let Err(err) = decoder.take(MAX_SIZE as u64 + 1).read_to_end(&mut list) {
            log::warn!("failed to read decompressed data: {err}");
            return Err(Error::DecompressionFailed);
        }
        if list.len() > MAX_SIZE {
            log::warn!("failed to read decompressed data: StreamTooLong");
            return Err(Error::DecompressionFailed);
        }

        self.data = list;

        // Make sure we note that our image is no longer compressed
        self.image.compression = TransmissionCompression::None;
        Ok(())
    }

    /// Decode the data as PNG. This will also update the image dimensions.
    fn decode_png(&mut self) -> Result<(), Error> {
        debug_assert_eq!(self.image.format, TransmissionFormat::Png);

        let (width, height, data) = decode_png_rgba(&self.data)?;
        if data.len() > MAX_SIZE {
            log::warn!(
                "png image too large size={} max_size={MAX_SIZE}",
                data.len()
            );
            return Err(Error::InvalidData);
        }

        // Replace our data
        self.data = data;

        // Store updated image dimensions
        self.image.width = width;
        self.image.height = height;
        self.image.format = TransmissionFormat::Rgba;
        Ok(())
    }
}

/// Decode a PNG into 8-bit non-premultiplied RGBA (what wuffs produces).
fn decode_png_rgba(data: &[u8]) -> Result<(u32, u32, Vec<u8>), Error> {
    let mut decoder = png::Decoder::new(Cursor::new(data));
    decoder.set_limits(png::Limits { bytes: MAX_SIZE });
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(|_| Error::InvalidData)?;
    let size = reader.output_buffer_size().ok_or(Error::InvalidData)?;
    if size > MAX_SIZE {
        return Err(Error::InvalidData);
    }
    let mut buf = vec![0u8; size];
    let info = reader
        .next_frame(&mut buf)
        .map_err(|_| Error::InvalidData)?;
    if info.bit_depth != png::BitDepth::Eight {
        return Err(Error::InvalidData);
    }
    let px = (info.width as usize)
        .checked_mul(info.height as usize)
        .ok_or(Error::InvalidData)?;
    let src = &buf[..info.buffer_size()];
    let mut out = Vec::with_capacity(px * 4);
    match info.color_type {
        png::ColorType::Rgba => out.extend_from_slice(src),
        png::ColorType::Rgb => {
            for c in src.as_chunks::<3>().0 {
                out.extend_from_slice(&[c[0], c[1], c[2], 255]);
            }
        }
        png::ColorType::Grayscale => {
            for &g in src {
                out.extend_from_slice(&[g, g, g, 255]);
            }
        }
        png::ColorType::GrayscaleAlpha => {
            for c in src.as_chunks::<2>().0 {
                out.extend_from_slice(&[c[0], c[0], c[0], c[1]]);
            }
        }
        png::ColorType::Indexed => return Err(Error::InvalidData),
    }
    if out.len() != px * 4 {
        return Err(Error::InvalidData);
    }
    Ok((info.width, info.height, out))
}

fn contains_subslice(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

/// Returns true if path appears to be in a temporary directory.
/// Copies logic from Kitty.
fn is_path_in_temp_dir(path: &[u8]) -> bool {
    is_path_in_temp_dir_with(path, alloc_tmp_dir())
}

/// [`is_path_in_temp_dir`] with an explicit temporary directory (an OS
/// string: it may be non-UTF-8, so it is compared byte for byte).
fn is_path_in_temp_dir_with(path: &[u8], tmp_dir: Option<std::ffi::OsString>) -> bool {
    if path.starts_with(b"/tmp") {
        return true;
    }
    if path.starts_with(b"/dev/shm") {
        return true;
    }
    if let Some(dir) = tmp_dir {
        if path.starts_with(dir.as_os_str().as_bytes()) {
            return true;
        }

        // The temporary dir is sometimes a symlink. On macOS for
        // example /tmp is /private/var/...
        if let Ok(real_dir) = realpath(&dir)
            && path.starts_with(real_dir.as_os_str().as_bytes())
        {
            return true;
        }
    }
    false
}

/// `os/file.zig allocTmpDir` on POSIX (TMPDIR, TMP, then /tmp), as the raw
/// OS string.
#[cfg(not(windows))]
#[allow(clippy::unnecessary_wraps)]
fn alloc_tmp_dir() -> Option<std::ffi::OsString> {
    for key in ["TMPDIR", "TMP"] {
        if let Some(v) = std::env::var_os(key) {
            return Some(v);
        }
    }
    Some("/tmp".into())
}

/// `os/file.zig allocTmpDir` on Windows (`TMP` only).
#[cfg(windows)]
fn alloc_tmp_dir() -> Option<std::ffi::OsString> {
    std::env::var_os("TMP")
}

/// Image represents a single fully loaded image.
#[derive(Debug, Clone)]
pub struct Image {
    pub id: u32,
    pub number: u32,
    pub width: u32,
    pub height: u32,
    pub format: TransmissionFormat,
    pub compression: TransmissionCompression,
    pub data: Vec<u8>,
    pub transmit_time: Instant,

    /// Set this to true if this image was loaded by a command that
    /// doesn't specify an ID or number, since such commands should
    /// not be responded to, even though we do currently give them
    /// IDs in the public range (which is bad!).
    pub implicit_id: bool,
}

impl Default for Image {
    fn default() -> Self {
        Image {
            id: 0,
            number: 0,
            width: 0,
            height: 0,
            format: TransmissionFormat::Rgb,
            compression: TransmissionCompression::None,
            data: Vec::new(),
            transmit_time: Instant::now(),
            implicit_id: false,
        }
    }
}

impl Image {
    /// Mostly for logging
    pub fn without_data(&self) -> Image {
        let mut copy = self.clone();
        copy.data = Vec::new();
        copy
    }
}

/// The rect taken up by some image placement, in grid cells. This will
/// be rounded up to the nearest grid cell since we can't place images
/// in partial grid cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub top_left: Pin,
    pub bottom_right: Pin,
}

#[cfg(test)]
mod tests {
    #[test]
    fn payload_path_bytes_are_kept() {
        // Unix keeps arbitrary bytes; Windows accepts WTF-8 (lone
        // surrogates) and rejects malformed input instead of using an
        // empty path.
        #[cfg(unix)]
        {
            let p = super::os_path_from_bytes(b"/tmp/\xff-tty-graphics-protocol").unwrap();
            assert_eq!(p.as_encoded_bytes(), b"/tmp/\xff-tty-graphics-protocol");
        }
        #[cfg(windows)]
        {
            assert!(super::os_path_from_bytes(b"C:\\\xed\xa0\x80").is_some());
            assert!(super::os_path_from_bytes(b"C:\\\xff").is_none());
        }
        assert!(super::os_path_from_bytes(b"plain").is_some());
    }

    #[cfg(unix)]
    #[test]
    fn temp_dir_check_keeps_non_utf8_tmpdir_bytes() {
        use std::os::unix::ffi::OsStringExt;
        let dir = std::ffi::OsString::from_vec(b"/data/t\xffmp".to_vec());
        assert!(is_path_in_temp_dir_with(
            b"/data/t\xffmp/tty-graphics-protocol-x",
            Some(dir.clone())
        ));
        // A lossy conversion would have turned the byte into U+FFFD.
        assert!(!is_path_in_temp_dir_with(
            b"/data/t\xef\xbf\xbdmp/tty-graphics-protocol-x",
            Some(dir)
        ));
        assert!(!is_path_in_temp_dir_with(b"/data/x", None));
    }

    use ghostty_vt::kitty::graphics_command::{Control, Transmission};

    use super::*;

    #[cfg(debug_assertions)]
    #[test]
    fn loading_image_debug_dump_writes_data() {
        let mut li = LoadingImage {
            image: Image {
                id: 7,
                width: 3,
                height: 2,
                format: TransmissionFormat::GrayAlpha,
                compression: TransmissionCompression::ZlibDeflate,
                ..Image::default()
            },
            data: vec![1, 2, 3, 4],
            display: None,
            quiet: Quiet::default(),
        };
        assert_eq!(
            "image-gray_alpha-zlib_deflate-3x2-7.data",
            li.debug_dump_filename()
        );
        let dir = std::env::temp_dir().join(format!("ghostty-dump-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        li.debug_dump_to(&dir).unwrap();
        let got = std::fs::read(dir.join(li.debug_dump_filename())).unwrap();
        assert_eq!(vec![1, 2, 3, 4], got);
        li.image.format = TransmissionFormat::Png;
        li.image.compression = TransmissionCompression::None;
        assert_eq!("image-png-none-3x2-7.data", li.debug_dump_filename());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    const RGB_NONE_20X15: &[u8] =
        include_bytes!("testdata/image-rgb-none-20x15-2147483647-raw.data");
    const RGB_ZLIB_128X96: &[u8] =
        include_bytes!("testdata/image-rgb-zlib_deflate-128x96-2147483647-raw.data");
    const PNG_50X76: &[u8] = include_bytes!("testdata/image-png-none-50x76-2147483647-raw.data");

    fn cmd(t: Transmission, data: &[u8]) -> Command {
        Command {
            control: Control::Transmit(t),
            quiet: Quiet::No,
            data: data.to_vec(),
        }
    }

    /// Unique temporary directory removed on drop.
    struct TmpDir(std::path::PathBuf);
    impl TmpDir {
        fn new(name: &str) -> TmpDir {
            let p =
                std::env::temp_dir().join(format!("ghostty-kitty-{}-{name}", std::process::id()));
            std::fs::create_dir_all(&p).unwrap();
            TmpDir(p)
        }
        fn write(&self, name: &str, data: &[u8]) -> String {
            let p = self.0.join(name);
            std::fs::write(&p, data).unwrap();
            realpath(p).unwrap().to_str().unwrap().to_owned()
        }
    }
    impl Drop for TmpDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    // This specifically tests we ALLOW invalid RGB data because Kitty
    // documents that this should work.
    #[test]
    fn image_load_with_invalid_rgb_data() {
        // <ESC>_Gi=31,s=1,v=1,a=q,t=d,f=24;AAAA<ESC>\
        let c = cmd(
            Transmission {
                format: TransmissionFormat::Rgb,
                width: 1,
                height: 1,
                image_id: 31,
                ..Transmission::default()
            },
            b"AAAA",
        );
        let _loading = LoadingImage::init(&c).unwrap();
    }

    #[test]
    fn image_load_with_image_too_wide() {
        let c = cmd(
            Transmission {
                format: TransmissionFormat::Rgb,
                width: MAX_DIMENSION + 1,
                height: 1,
                image_id: 31,
                ..Transmission::default()
            },
            b"AAAA",
        );
        let mut loading = LoadingImage::init(&c).unwrap();
        assert_eq!(loading.complete().unwrap_err(), Error::DimensionsTooLarge);
    }

    #[test]
    fn image_load_with_image_too_tall() {
        let c = cmd(
            Transmission {
                format: TransmissionFormat::Rgb,
                height: MAX_DIMENSION + 1,
                width: 1,
                image_id: 31,
                ..Transmission::default()
            },
            b"AAAA",
        );
        let mut loading = LoadingImage::init(&c).unwrap();
        assert_eq!(loading.complete().unwrap_err(), Error::DimensionsTooLarge);
    }

    #[test]
    fn image_load_rgb_zlib_compressed_direct() {
        let c = cmd(
            Transmission {
                format: TransmissionFormat::Rgb,
                medium: TransmissionMedium::Direct,
                compression: TransmissionCompression::ZlibDeflate,
                height: 96,
                width: 128,
                image_id: 31,
                ..Transmission::default()
            },
            RGB_ZLIB_128X96,
        );
        let mut loading = LoadingImage::init(&c).unwrap();
        let img = loading.complete().unwrap();

        // should be decompressed
        assert_eq!(img.compression, TransmissionCompression::None);
        assert_eq!(img.data.len(), 128 * 96 * 3);
    }

    #[test]
    fn image_load_rgb_not_compressed_direct() {
        let c = cmd(
            Transmission {
                format: TransmissionFormat::Rgb,
                medium: TransmissionMedium::Direct,
                compression: TransmissionCompression::None,
                width: 20,
                height: 15,
                image_id: 31,
                ..Transmission::default()
            },
            RGB_NONE_20X15,
        );
        let mut loading = LoadingImage::init(&c).unwrap();
        let img = loading.complete().unwrap();
        assert_eq!(img.compression, TransmissionCompression::None);
    }

    #[test]
    fn image_load_rgb_zlib_compressed_direct_chunked() {
        let data = RGB_ZLIB_128X96;
        let c = cmd(
            Transmission {
                format: TransmissionFormat::Rgb,
                medium: TransmissionMedium::Direct,
                compression: TransmissionCompression::ZlibDeflate,
                height: 96,
                width: 128,
                image_id: 31,
                more_chunks: true,
                ..Transmission::default()
            },
            &data[0..1024],
        );
        let mut loading = LoadingImage::init(&c).unwrap();

        // Read our remaining chunks
        for chunk in data[1024..].chunks(1024) {
            loading.add_data(chunk).unwrap();
        }

        let img = loading.complete().unwrap();
        assert_eq!(img.compression, TransmissionCompression::None);
    }

    #[test]
    fn image_load_rgb_zlib_compressed_direct_chunked_with_zero_initial_chunk() {
        let data = RGB_ZLIB_128X96;
        let c = cmd(
            Transmission {
                format: TransmissionFormat::Rgb,
                medium: TransmissionMedium::Direct,
                compression: TransmissionCompression::ZlibDeflate,
                height: 96,
                width: 128,
                image_id: 31,
                more_chunks: true,
                ..Transmission::default()
            },
            b"",
        );
        let mut loading = LoadingImage::init(&c).unwrap();
        for chunk in data.chunks(1024) {
            loading.add_data(chunk).unwrap();
        }
        let img = loading.complete().unwrap();
        assert_eq!(img.compression, TransmissionCompression::None);
    }

    #[test]
    fn image_load_temporary_file_without_correct_path() {
        let tmp = TmpDir::new("nocorrectpath");
        let path = tmp.write("image.data", RGB_NONE_20X15);
        let c = cmd(
            Transmission {
                format: TransmissionFormat::Rgb,
                medium: TransmissionMedium::TemporaryFile,
                compression: TransmissionCompression::None,
                width: 20,
                height: 15,
                image_id: 31,
                ..Transmission::default()
            },
            path.as_bytes(),
        );
        assert_eq!(
            LoadingImage::init(&c).unwrap_err(),
            Error::TemporaryFileNotNamedCorrectly
        );

        // Temporary file should still be there
        assert!(Path::new(&path).exists());
    }

    #[test]
    fn image_load_rgb_not_compressed_temporary_file() {
        let tmp = TmpDir::new("tempfile");
        let path = tmp.write("tty-graphics-protocol-image.data", RGB_NONE_20X15);
        let c = cmd(
            Transmission {
                format: TransmissionFormat::Rgb,
                medium: TransmissionMedium::TemporaryFile,
                compression: TransmissionCompression::None,
                width: 20,
                height: 15,
                image_id: 31,
                ..Transmission::default()
            },
            path.as_bytes(),
        );
        let mut loading = LoadingImage::init(&c).unwrap();
        let img = loading.complete().unwrap();
        assert_eq!(img.compression, TransmissionCompression::None);

        // Temporary file should be gone
        assert!(!Path::new(&path).exists());
    }

    #[test]
    fn image_load_rgb_not_compressed_regular_file() {
        let tmp = TmpDir::new("regularfile");
        let path = tmp.write("image.data", RGB_NONE_20X15);
        let c = cmd(
            Transmission {
                format: TransmissionFormat::Rgb,
                medium: TransmissionMedium::File,
                compression: TransmissionCompression::None,
                width: 20,
                height: 15,
                image_id: 31,
                ..Transmission::default()
            },
            path.as_bytes(),
        );
        let mut loading = LoadingImage::init(&c).unwrap();
        let img = loading.complete().unwrap();
        assert_eq!(img.compression, TransmissionCompression::None);
        assert!(Path::new(&path).exists());
    }

    #[test]
    fn image_load_png_not_compressed_regular_file() {
        let tmp = TmpDir::new("pngfile");
        let path = tmp.write("tty-graphics-protocol-image.data", PNG_50X76);
        let c = cmd(
            Transmission {
                format: TransmissionFormat::Png,
                medium: TransmissionMedium::File,
                compression: TransmissionCompression::None,
                width: 0,
                height: 0,
                image_id: 31,
                ..Transmission::default()
            },
            path.as_bytes(),
        );
        let mut loading = LoadingImage::init(&c).unwrap();
        let img = loading.complete().unwrap();
        assert_eq!(img.compression, TransmissionCompression::None);
        assert_eq!(img.format, TransmissionFormat::Rgba);
        assert_eq!((img.width, img.height), (50, 76));
        assert_eq!(img.data.len(), 50 * 76 * 4);
        assert!(Path::new(&path).exists());
    }

    #[test]
    fn image_load_rejects_proc_and_nul_paths() {
        // The `/proc/` guard is a prefix check on the canonical path, which
        // on Windows is a drive path (Wine even maps `/proc` through `Z:`),
        // so it only applies to POSIX.
        #[cfg(unix)]
        {
            let c = cmd(
                Transmission {
                    format: TransmissionFormat::Rgb,
                    medium: TransmissionMedium::File,
                    width: 1,
                    height: 1,
                    ..Transmission::default()
                },
                b"/proc/self/status",
            );
            assert_eq!(LoadingImage::init(&c).unwrap_err(), Error::InvalidData);
        }
        let c = cmd(
            Transmission {
                medium: TransmissionMedium::File,
                ..Transmission::default()
            },
            b"/tmp/a\0b",
        );
        assert_eq!(LoadingImage::init(&c).unwrap_err(), Error::InvalidData);
    }

    #[test]
    fn image_load_file_offset_and_size() {
        let tmp = TmpDir::new("offsetsize");
        let mut data = vec![0xAAu8; 5];
        data.extend_from_slice(&[1, 2, 3, 4, 5, 6]);
        data.extend_from_slice(&[0xBB; 7]);
        let path = tmp.write("image.data", &data);
        let c = cmd(
            Transmission {
                format: TransmissionFormat::Rgb,
                medium: TransmissionMedium::File,
                width: 2,
                height: 1,
                offset: 5,
                size: 6,
                ..Transmission::default()
            },
            path.as_bytes(),
        );
        let mut loading = LoadingImage::init(&c).unwrap();
        assert_eq!(loading.data, vec![1, 2, 3, 4, 5, 6]);
        assert!(loading.complete().is_ok());
    }

    #[cfg(not(any(target_arch = "wasm32", windows)))]
    #[test]
    fn image_load_shared_memory() {
        use std::io::Write;

        use rustix::{fs::Mode, shm};

        let name = format!("/ghostty-kitty-test-{}", std::process::id());
        let fd = shm::open(
            name.as_str(),
            shm::OFlags::CREATE | shm::OFlags::EXCL | shm::OFlags::RDWR,
            Mode::RUSR | Mode::WUSR,
        )
        .unwrap();
        let mut f = std::fs::File::from(fd);
        f.write_all(&[7u8; 20 * 15 * 3]).unwrap();
        drop(f);

        let c = cmd(
            Transmission {
                format: TransmissionFormat::Rgb,
                medium: TransmissionMedium::SharedMemory,
                width: 20,
                height: 15,
                ..Transmission::default()
            },
            name.as_bytes(),
        );
        let mut loading = LoadingImage::init(&c).unwrap();
        let img = loading.complete().unwrap();
        assert_eq!(img.data.len(), 900);
        assert!(img.data.iter().all(|&b| b == 7));

        // The segment is unlinked after loading.
        assert!(shm::open(name.as_str(), shm::OFlags::RDONLY, Mode::empty()).is_err());
    }
}
