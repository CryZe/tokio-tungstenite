use std::io::{Read, Write};
use std::pin::Pin;
use std::task::{Context, Poll};

use tokio_io::{AsyncRead, AsyncWrite};
use tungstenite::{Error as WsError, WebSocket};

pub(crate) trait HasContext {
    fn set_context(&mut self, context: *mut ());
}
#[derive(Debug)]
pub struct AllowStd<S> {
    pub(crate) inner: S,
    pub(crate) context: *mut (),
}

impl<S> HasContext for AllowStd<S> {
    fn set_context(&mut self, context: *mut ()) {
        self.context = context;
    }
}

pub(crate) struct Guard<'a, S>(pub(crate) &'a mut WebSocket<AllowStd<S>>);

impl<S> Drop for Guard<'_, S> {
    fn drop(&mut self) {
        (self.0).get_mut().context = std::ptr::null_mut();
    }
}

// *mut () context is neither Send nor Sync
unsafe impl<S: Send> Send for AllowStd<S> {}
unsafe impl<S: Sync> Sync for AllowStd<S> {}

impl<S> AllowStd<S>
    where
        S: Unpin,
{
    fn with_context<F, R>(&mut self, f: F) -> R
        where
            F: FnOnce(&mut Context<'_>, Pin<&mut S>) -> R,
    {
        unsafe {
            assert!(!self.context.is_null());
            let waker = &mut *(self.context as *mut _);
            f(waker, Pin::new(&mut self.inner))
        }
    }

    pub(crate) fn get_mut(&mut self) -> &mut S {
        &mut self.inner
    }

    pub(crate) fn get_ref(&self) -> &S {
        &self.inner
    }
}

impl<S> Read for AllowStd<S>
    where
        S: AsyncRead + Unpin,
{
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self.with_context(|ctx, stream| stream.poll_read(ctx, buf)) {
            Poll::Ready(r) => r,
            Poll::Pending => Err(std::io::Error::from(std::io::ErrorKind::WouldBlock)),
        }
    }
}

impl<S> Write for AllowStd<S>
    where
        S: AsyncWrite + Unpin,
{
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self.with_context(|ctx, stream| stream.poll_write(ctx, buf)) {
            Poll::Ready(r) => r,
            Poll::Pending => Err(std::io::Error::from(std::io::ErrorKind::WouldBlock)),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self.with_context(|ctx, stream| stream.poll_flush(ctx)) {
            Poll::Ready(r) => r,
            Poll::Pending => Err(std::io::Error::from(std::io::ErrorKind::WouldBlock)),
        }
    }
}

pub(crate) fn cvt<T>(r: Result<T, WsError>) -> Poll<Result<T, WsError>> {
    match r {
        Ok(v) => Poll::Ready(Ok(v)),
        Err(WsError::Io(ref e)) if e.kind() == std::io::ErrorKind::WouldBlock => Poll::Pending,
        Err(e) => Poll::Ready(Err(e)),
    }
}