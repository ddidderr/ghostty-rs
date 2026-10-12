//! A model of Zig 0.15 `std.fs.File.writer(&buffer).interface`.
//!
//! Zig actions write through a fixed-size buffer that is only written to the
//! file when it overflows or when the action calls `flush`, so two writers on
//! the same file (as in `+show-face`) interleave in flush order, not call
//! order. This type keeps that ordering.
//!
//! Two upstream data-loss behaviours are deliberately NOT replicated (see
//! `docs/rewrite/notes/final-Z17.md`): Zig's positional `pwrite` at the
//! writer's own offset 0 (writers overwrite each other and existing content
//! when stdout is a regular file) and buffers that are never flushed. Data is
//! always appended at the current end and anything still buffered is flushed
//! when the writer is dropped.

#[cfg(unix)]
use std::os::fd::AsFd;
use std::{
    cell::RefCell,
    io::{self, Write},
    rc::Rc,
};

/// Where a [`ZigWriter`] drains to.
pub enum Target {
    /// A duplicate of a standard descriptor.
    #[cfg(unix)]
    Fd(std::fs::File),
    /// A plain stream.
    Stream(Box<dyn Write>),
}

/// A shared output target so several [`ZigWriter`]s can drain to one file.
pub type Sink = Rc<RefCell<Target>>;

/// Standard output as a [`Sink`].
#[must_use]
pub fn stdout_sink() -> Sink {
    #[cfg(unix)]
    if let Ok(fd) = io::stdout().as_fd().try_clone_to_owned() {
        return Rc::new(RefCell::new(Target::Fd(fd.into())));
    }
    Rc::new(RefCell::new(Target::Stream(Box::new(io::stdout()))))
}

/// Standard error as a [`Sink`].
#[must_use]
pub fn stderr_sink() -> Sink {
    #[cfg(unix)]
    if let Ok(fd) = io::stderr().as_fd().try_clone_to_owned() {
        return Rc::new(RefCell::new(Target::Fd(fd.into())));
    }
    Rc::new(RefCell::new(Target::Stream(Box::new(io::stderr()))))
}

/// A sink over any stream (tests).
#[cfg(test)]
#[must_use]
pub fn stream_sink(w: impl Write + 'static) -> Sink {
    Rc::new(RefCell::new(Target::Stream(Box::new(w))))
}

/// `File.Writer` with a `cap` byte buffer.
pub struct ZigWriter {
    buf: Vec<u8>,
    cap: usize,
    sink: Sink,
}

impl ZigWriter {
    #[must_use]
    pub fn new(sink: Sink, cap: usize) -> Self {
        Self {
            buf: Vec::with_capacity(cap),
            cap,
            sink,
        }
    }

    /// `File.Writer.end`: a flush (Zig also truncates; not replicated).
    pub fn end(&mut self) -> io::Result<()> {
        self.flush()
    }

    fn drain(&mut self, extra: &[u8]) -> io::Result<()> {
        let mut pending = std::mem::take(&mut self.buf);
        pending.extend_from_slice(extra);
        if pending.is_empty() {
            return Ok(());
        }
        match &mut *self.sink.borrow_mut() {
            #[cfg(unix)]
            Target::Fd(file) => file.write_all(&pending),
            Target::Stream(w) => {
                w.write_all(&pending)?;
                w.flush()
            }
        }
    }
}

impl Drop for ZigWriter {
    fn drop(&mut self) {
        let _ = self.flush();
    }
}

impl Write for ZigWriter {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.write_all(data)?;
        Ok(data.len())
    }

    /// Zig `Writer.write`: data that fits is buffered; otherwise the drain
    /// writes the buffered bytes followed by the data straight to the file.
    fn write_all(&mut self, data: &[u8]) -> io::Result<()> {
        if self.buf.len() + data.len() <= self.cap {
            self.buf.extend_from_slice(data);
            return Ok(());
        }
        self.drain(data)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.drain(&[])
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[derive(Clone, Default)]
    struct Mem(Rc<RefCell<Vec<u8>>>);
    impl Write for Mem {
        fn write(&mut self, d: &[u8]) -> io::Result<usize> {
            self.0.borrow_mut().extend_from_slice(d);
            Ok(d.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn buffers_until_flush_in_flush_order() {
        let mem = Mem::default();
        let sink: Sink = stream_sink(mem.clone());
        let mut a = ZigWriter::new(sink.clone(), 1024);
        let mut b = ZigWriter::new(sink, 1024);
        b.write_all(b"err\n").unwrap();
        a.write_all(b"out\n").unwrap();
        assert!(mem.0.borrow().is_empty());
        a.flush().unwrap();
        b.flush().unwrap();
        assert_eq!(&*mem.0.borrow(), b"out\nerr\n");
    }

    #[test]
    fn overflow_drains_buffer_then_data() {
        let mem = Mem::default();
        let sink: Sink = stream_sink(mem.clone());
        let mut w = ZigWriter::new(sink, 4);
        w.write_all(b"ab").unwrap();
        w.write_all(b"cdefg").unwrap();
        assert_eq!(&*mem.0.borrow(), b"abcdefg");
        w.write_all(b"h").unwrap();
        assert_eq!(&*mem.0.borrow(), b"abcdefg");
    }

    #[cfg(unix)]
    #[test]
    fn regular_file_output_is_complete_and_appended() {
        let mut tmp = tempfile::tempfile().unwrap();
        std::io::Write::write_all(&mut tmp, b"pre\n").unwrap();
        let sink: Sink = Rc::new(RefCell::new(Target::Fd(tmp.try_clone().unwrap())));
        let mut a = ZigWriter::new(sink.clone(), 1024);
        let mut b = ZigWriter::new(sink, 1024);
        a.write_all(b"0123456789\n").unwrap();
        b.write_all(b"abc\n").unwrap();
        a.flush().unwrap();
        b.flush().unwrap();
        let mut out = String::new();
        std::io::Seek::rewind(&mut tmp).unwrap();
        std::io::Read::read_to_string(&mut tmp, &mut out).unwrap();
        assert_eq!(out, "pre\n0123456789\nabc\n");
    }

    #[test]
    fn unflushed_buffer_is_flushed_on_drop() {
        let mem = Mem::default();
        let sink: Sink = stream_sink(mem.clone());
        let mut w = ZigWriter::new(sink, 4096);
        w.write_all(b"kept").unwrap();
        drop(w);
        assert_eq!(&*mem.0.borrow(), b"kept");
    }
}
