use crate::TcpStream;
use crate::net::PROVIDER;
use crate::runtime::AVAILABLE_PARALLELISM;
use futures_util::{AsyncRead, AsyncWrite};
use std::io::{Error, ErrorKind};
use std::io::{Read, Write};
use std::net::Shutdown;
use std::pin::Pin;
use std::task::{Context, Poll};

impl AsyncRead for TcpStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<Result<usize, Error>> {
        let provider = if let Some(provider) = PROVIDER.get() {
            provider
        } else {
            cx.waker().wake_by_ref();
            return Poll::Pending;
        };

        let slab = provider.give_slab();

        // We need to register the waker if `will_wake returns false` because if `poll_read`
        // returns Poll::Ready, the underlying future created by the method on the extension traits
        // like AsyncReadExt might get dropped and they might create a new one on the next call and
        // if we do not update the waker, our call will hang!

        // The stream was already registered by the `StreamConnectFuture`!
        // We subtract (AVAILABLE_PARALLELISM + 1) because we added it while registering the stream!
        slab.update(
            self.token.0 - (AVAILABLE_PARALLELISM + 1),
            cx.waker().clone(),
        );

        match self.stream.read(buf) {
            Ok(n) => Poll::Ready(Ok(n)),
            // Handling errors as per the documentation in the futures_util crate!
            Err(e) if e.kind() == ErrorKind::WouldBlock => Poll::Pending,
            Err(e) if e.kind() == ErrorKind::Interrupted => {
                cx.waker().wake_by_ref();
                Poll::Pending
            }
            Err(e) => Poll::Ready(Err(Error::other(e))),
        }
    }
}

impl AsyncWrite for TcpStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, Error>> {
        let provider = if let Some(provider) = PROVIDER.get() {
            provider
        } else {
            cx.waker().wake_by_ref();
            return Poll::Pending;
        };

        let slab = provider.give_slab();

        // The stream was already registered by the `StreamConnectFuture`!
        slab.update(
            self.token.0 - (AVAILABLE_PARALLELISM + 1),
            cx.waker().clone(),
        );

        match self.stream.write(buf) {
            Ok(n) => Poll::Ready(Ok(n)),
            Err(e) if e.kind() == ErrorKind::WouldBlock => Poll::Pending,
            Err(e) if e.kind() == ErrorKind::Interrupted => {
                cx.waker().wake_by_ref();
                Poll::Pending
            }
            Err(e) => Poll::Ready(Err(Error::other(e))),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), Error>> {
        Poll::Ready(Ok(()))
    }

    // We attempt to shutdown use `mio::TcpStream::shutdown`. If other tasks are also using the same
    // stream, they will not hang as the OS will notify all registered wakers which in all cases in
    // this runtime is just one that the socket is being closed and the reactor will wake the
    // appropriate wakers up!
    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
        let provider = if let Some(provider) = PROVIDER.get() {
            provider
        } else {
            cx.waker().wake_by_ref();
            return Poll::Pending;
        };

        let slab = provider.give_slab();

        // The stream was already registered by the `StreamConnectFuture`!
        slab.update(
            self.token.0 - (AVAILABLE_PARALLELISM + 1),
            cx.waker().clone(),
        );

        // We try to shutdown the Write half of the connection!
        match self.stream.shutdown(Shutdown::Write) {
            Ok(_) => Poll::Ready(Ok(())),
            Err(e) if e.kind() == ErrorKind::WouldBlock => Poll::Pending,
            Err(e) if e.kind() == ErrorKind::Interrupted => {
                cx.waker().wake_by_ref();
                Poll::Pending
            }
            Err(e) => Poll::Ready(Err(Error::other(e))),
        }
    }
}
