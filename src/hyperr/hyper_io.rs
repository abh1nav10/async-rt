use crate::net::TcpStream;
use futures_util::{AsyncRead, AsyncWrite};
use hyper::rt::{Read, ReadBufCursor, Write};
use std::io::Error;
use std::mem::MaybeUninit;
use std::pin::Pin;
use std::task::{Context, Poll};

impl Read for TcpStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        mut buf: ReadBufCursor<'_>,
    ) -> Poll<Result<(), Error>> {
        // Casting from *mut MaybeUninit<u8> to *mut [u8] to make it compatible with AsyncRead's
        // API
        //
        // # SAFETY:
        //     We get the number of bytes read from `AsyncRead::poll_read` and then advance the
        //     ReadBufCursor to exactly those many bytes thus making the operations performed by
        //     the `hyper` crate on those bytes safe! Hence, this cast in and of itself does not
        //     directly try to read from the bytes in the buffer. It just passes the buffer into
        //     `AsyncRead::poll_read` which WRITES into the buffer and tells us how many bytes have
        //     been written!
        let buffer =
            unsafe { std::mem::transmute::<&mut [MaybeUninit<u8>], &mut [u8]>(buf.as_mut()) };

        match AsyncRead::poll_read(self, cx, buffer) {
            Poll::Ready(Ok(n)) => {
                unsafe { buf.advance(n) };
                Poll::Ready(Ok(()))
            }
            Poll::Ready(Err(e)) => Poll::Ready(Err(e)),
            Poll::Pending => Poll::Pending,
        }
    }
}

impl Write for TcpStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, Error>> {
        AsyncWrite::poll_write(self, cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
        AsyncWrite::poll_flush(self, cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
        AsyncWrite::poll_close(self, cx)
    }
}
